#![cfg(feature = "database-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "isolated database regression fixture"
)]

use persistence::{Database, DatabaseConfig, FrontMatterRepository};
use serde_json::json;
use sqlx::PgPool;
use std::{env, time::Duration};
use uuid::Uuid;

#[tokio::test]
async fn department_projection_uses_institutional_ids_and_names() {
    let url = env::var("TEST_DATABASE_URL").expect("isolated TEST_DATABASE_URL required");
    let database =
        Database::connect(DatabaseConfig::new(&url, 1, 5, Duration::from_secs(5)).unwrap())
            .await
            .unwrap();
    database.migrate().await.unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let user = Uuid::from_u128(3001);
    let tenant = Uuid::from_u128(3002);
    let workspace = Uuid::from_u128(3003);
    let paper = Uuid::from_u128(3004);
    let departments = [
        (
            Uuid::from_u128(3005),
            "PERSIST-CSE",
            "Computer Science and Engineering",
        ),
        (
            Uuid::from_u128(3006),
            "PERSIST-ECE",
            "Electronics and Communication Engineering",
        ),
    ];
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO latex_core.tenants(id) VALUES($1)")
        .bind(tenant)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.users(id,tenant_id) VALUES($1,$2)")
        .bind(user)
        .bind(tenant)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.user_credentials(user_id,email,password_hash) VALUES($1,'department-proof@persistence.example','test-hash')").bind(user).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO latex_core.global_user_roles(user_id,role) VALUES($1,'writer')")
        .bind(user)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspaces(id,tenant_id,owner_user_id) VALUES($1,$2,$3)")
        .bind(workspace)
        .bind(tenant)
        .bind(user)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.paper_teams(id,workspace_id,name,created_by_user_id) VALUES($1,$2,'Department authority',$3)").bind(paper).bind(workspace).bind(user).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO latex_core.paper_team_members(paper_team_id,user_id,assigned_by_user_id,is_leader) VALUES($1,$2,$2,true)").bind(paper).bind(user).execute(&mut *tx).await.unwrap();
    for (id, code, name) in departments {
        sqlx::query("INSERT INTO vcap.departments(department_id,department_name) VALUES($1,$2)")
            .bind(id)
            .bind(name)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO vcap.faculty(faculty_id,name,dept_id) VALUES($1,'Programme HOD',$2)",
        )
        .bind(code)
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("INSERT INTO vcap.programmes(programme_code,hod_id,programme_name) VALUES($1,$1,'Identical programme name')").bind(code).execute(&mut *tx).await.unwrap();
    }
    sqlx::query("INSERT INTO vcap.students(reg_no,name,programme_code) VALUES('PERSIST-DEPT-STUDENT','Department Student',$1)").bind(departments[0].1).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO vcap.student_user_links(reg_no,user_id,status,linked_at) VALUES('PERSIST-DEPT-STUDENT',$1,'LINKED',now())").bind(user).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO latex_core.paper_project_metadata(paper_team_id,department_display_names,updated_by_user_id) VALUES($1,$2,$3)").bind(paper).bind(json!([{"id":departments[0].0,"display_name":"Untrusted team name"}])).bind(user).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let repo = FrontMatterRepository::new(database.clone());
    for (id, code, name) in departments {
        sqlx::query(
            "UPDATE vcap.students SET programme_code=$1 WHERE reg_no='PERSIST-DEPT-STUDENT'",
        )
        .bind(code)
        .execute(&pool)
        .await
        .unwrap();
        let values = repo.automatic_values(paper).await.unwrap();
        assert_eq!(values["department.id"], id.to_string());
        assert_eq!(values["department_name"], name);
        let metadata = repo.project_metadata(paper).await.unwrap();
        let entry = metadata["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["key"] == "departments")
            .unwrap();
        assert_eq!(entry["value"][0]["id"], id.to_string());
        assert_eq!(entry["value"][0]["display_name"], name);
        assert_eq!(entry["value"][0]["origin"], "database");
    }
    // Nullable institutional names stay unresolved, without using old team labels.
    sqlx::query("UPDATE vcap.departments SET department_name=NULL WHERE department_id=$1")
        .bind(departments[1].0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !repo
            .automatic_values(paper)
            .await
            .unwrap()
            .contains_key("department_name")
    );
    let metadata = repo.project_metadata(paper).await.unwrap();
    let entry = metadata["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "departments")
        .unwrap();
    assert!(entry["value"][0]["display_name"].is_null());
    pool.close().await;
    database.close().await;
}
