// Disposable database, actual worker and browser; no fabricated build state.
#[tokio::test]
async fn review_regressions_browser_acceptance() {
    use sqlx::Row as _;
    let _guard = SERVER_TEST_LOCK.lock().await;
    tracing_subscriber::fmt()
        .with_env_filter("off,review_metadata_failure=error,latex_core_api::review_api=info,latex_core_api::review_diagnostics=warn")
        .with_test_writer()
        .try_init()
        .expect("acceptance tracing initializes once");
    let (database, pool, _app, _storage, mut state) = test_application().await;
    state.mail_enabled = false;
    state.mail_outbox = None;
    let compiler = Arc::new(
        compiler::CompilerService::new(
            state.blobs.clone(),
            compiler::DockerCliRuntime::new(
                env::var("LATEX_CORE_COURSE_TEXLIVE_IMAGE").expect("frozen compiler required"),
            )
            .unwrap(),
            compiler::CompilerConfig::new(compiler::CompileLimits::development_default())
                .with_staging_root(_storage.path().join("compiler")),
        )
        .unwrap(),
    );
    state.environment = compiler.environment_id().clone();
    let app = router(state.clone());
    let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
    let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let other = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
    let outsider = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
    let writer_id = test_user_id(&pool, &writer.email).await;
    let other_id = test_user_id(&pool, &other.email).await;
    let mentor_id = test_user_id(&pool, &mentor.email).await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let registration = format!("R{}", &suffix[..8]);
    let other_registration = format!("W{}", &suffix[..8]);
    let programme = format!("P{suffix}");
    let hod = format!("H{suffix}");
    let dean = format!("D{suffix}");
    let guide = format!("G{suffix}");
    let school = format!("S{suffix}");
    let department = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO vcap.departments(department_id,department_name) VALUES($1,'Synthetic Computer Science')").bind(department).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.faculty(faculty_id,name,honorific,designation,dept_id) VALUES($1,'Synthetic Head','Dr.','Professor',$4),($2,'Synthetic Dean','Dr.','Professor',$4),($3,'Synthetic Guide','Dr.','Associate Professor',$4)").bind(&hod).bind(&dean).bind(&guide).bind(department).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.programmes(programme_code,hod_id,programme_name,degree_name,specialization) VALUES($1,$2,'Computer Science and Engineering','Bachelor of Technology','Intelligent Systems')").bind(&programme).bind(&hod).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.schools(school_id,school_name) VALUES($1,'Synthetic School of Computing')").bind(&school).execute(&pool).await.unwrap();
    for (faculty, role) in [(&hod, "HOD"), (&dean, "Dean"), (&guide, "Guide")] {
        sqlx::query("INSERT INTO vcap.faculty_roles(role_id,faculty_id,role_type,programme_code,school_id,status) VALUES($1,$2,$3,$4,$5,'ACTIVE')").bind(uuid::Uuid::new_v4()).bind(faculty).bind(role).bind(&programme).bind(&school).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO vcap.faculty_user_links(faculty_id,user_id,status,linked_at) VALUES($1,$2,'LINKED',now())").bind(&guide).bind(mentor_id.as_uuid()).execute(&pool).await.unwrap();
    for (reg, user, name, email) in [
        (
            &registration,
            writer_id,
            "Synthetic Review Leader",
            &writer.email,
        ),
        (
            &other_registration,
            other_id,
            "Synthetic Other Writer",
            &other.email,
        ),
    ] {
        sqlx::query(
            "INSERT INTO vcap.students(reg_no,name,email,programme_code) VALUES($1,$2,$3,$4)",
        )
        .bind(reg)
        .bind(name)
        .bind(email)
        .bind(&programme)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO vcap.student_user_links(reg_no,user_id,status,linked_at) VALUES($1,$2,'LINKED',now())").bind(reg).bind(user.as_uuid()).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO vcap.student_course_registrations(student_reg_no,course_id,academic_year,semester) VALUES($1,'BCSE4973','2026-2027','FALL')").bind(&registration).execute(&pool).await.unwrap();
    let main_name = "Full_Report_template_v1.0/Full_Report_v1.0.tex";
    let (body, content_type) = template_multipart(
        &[
            ("name", registration.as_str()),
            ("main", main_name),
            ("arrangement", "SINGLE_SOURCE"),
        ],
        include_bytes!("../../../artifacts/Full_Report_template_v1.1.zip"),
    );
    let imported = request_bytes(
        &app,
        Method::POST,
        "/api/admin/v2/templates/import",
        Some(&admin.cookie),
        body,
        Some(&content_type),
    )
    .await;
    assert_eq!(imported.status(), StatusCode::CREATED);
    let template = test_json(imported).await;
    state
        .institution
        .set_global_fallback(
            test_user_id(&pool, &admin.email).await,
            uuid::Uuid::parse_str(template["id"].as_str().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    let mut teams = Vec::new();
    for name in [
        "Review regression fixture",
        "Policy Team C",
        "Policy Team B",
    ] {
        let response = request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
            &serde_json::json!({"name":name,"writer_ids":[writer_id,other_id],"leader_writer_id":writer_id,"mentor_ids":[mentor_id],"template_id":template["id"],"use_front_matter_default":false}).to_string(), Some("application/json")).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        teams.push(test_json(response).await["team"]["id"].clone());
    }
    // Existing-Team metadata from the older contract must not override registration.
    for team in &teams {
        let id = uuid::Uuid::parse_str(team.as_str().unwrap()).unwrap();
        for (key, value) in [
            ("course_code", "BA101CSE101"),
            ("team_academic_year", "2025-2026"),
            ("team_semester", "Winter Semester"),
        ] {
            sqlx::query("INSERT INTO latex_core.paper_front_matter_values(paper_team_id,field_key,value_json,value_source,updated_by_user_id) VALUES($1,$2,$3,'TEAM_OVERRIDE',$4)").bind(id).bind(key).bind(serde_json::json!(value)).bind(writer_id.as_uuid()).execute(&pool).await.unwrap();
        }
    }
    if env::var_os("LATEX_CORE_FIDELITY_OUTPUT").is_some() {
        verify_actual_template_fidelity(&app, &state, &pool, &writer, &teams[1], main_name).await;
    }
    let port = env::var("REVIEW_PORT").unwrap_or_else(|_| "0".to_owned());
    let bind = env::var("REVIEW_BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let listener = tokio::net::TcpListener::bind(format!("{bind}:{port}")).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut deployed_api = None;
    let server = if let Some(executable) = env::var_os("LATEX_CORE_TEST_API_EXECUTABLE") {
        drop(listener);
        let mut child = tokio::process::Command::new(executable)
            .env("DATABASE_URL", env::var("TEST_DATABASE_URL").unwrap())
            .env("BLOB_STORAGE_ROOT", _storage.path())
            .env("TEX_ENVIRONMENT_ID", state.environment.as_str())
            .env("BIND_ADDR", format!("{bind}:{port}"))
            .env("SESSION_COOKIE_SECURE", "false")
            .env("ALLOW_REGISTRATION", "false")
            .env("LATEX_CORE_MAIL_ENABLED", "false")
            .env("LATEX_CORE_PUBLIC_BASE_URL", "http://canonical.example.invalid:9000")
            .env("RUST_LOG", "warn,latex_core_api::review_api=info")
            .kill_on_drop(true)
            .spawn().unwrap();
        let address = format!("127.0.0.1:{port}");
        let mut ready = false;
        for _ in 0..100 {
            assert!(child.try_wait().unwrap().is_none(), "exported API exited before readiness");
            if tokio::net::TcpStream::connect(&address).await.is_ok() { ready = true; break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ready, "exported API did not start");
        deployed_api = Some(child);
        None
    } else {
        Some(tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); }))
    };
    let worker = queue::CompilationWorker::new(
        state.queue.clone(),
        state.blobs.clone(),
        compiler,
        core_types::WorkerId::new(),
        queue::WorkerConfig::new(1, Duration::from_millis(100)).unwrap(),
    );
    let shutdown = queue::WorkerShutdown::new();
    let worker_shutdown = shutdown.clone();
    let worker_task = tokio::spawn(async move {
        worker.run_until_shutdown(worker_shutdown).await.unwrap();
    });
    let config = serde_json::json!({"port":port,"paperId":teams[0],"teamIds":teams,"mainPath":main_name,"writerCookie":writer.cookie,"otherCookie":other.cookie,"mentorCookie":mentor.cookie,"outsiderCookie":outsider.cookie,"adminCookie":admin.cookie,"writerEmail":writer.email,"otherEmail":other.email,"mentorEmail":mentor.email,"adminEmail":admin.email,"password":PASSWORD});
    let output = tokio::task::spawn_blocking(move || {
        Command::new("node")
            .arg(
                env::var("LATEX_CORE_REVIEW_BROWSER_SCRIPT")
                    .unwrap_or_else(|_| "tests/review-regressions-browser.mjs".to_owned()),
            )
            .env("LATEX_CORE_REVIEW_CONFIG", config.to_string())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .unwrap()
    })
    .await
    .unwrap();
    shutdown.request();
    worker_task.await.unwrap();
    if let Some(server) = server { server.abort(); }
    if let Some(mut child) = deployed_api { child.kill().await.unwrap(); child.wait().await.unwrap(); }
    assert!(output.success(), "review browser acceptance failed");
    if env::var_os("REVIEW_AUDIT_ONLY").is_some() {
        let paper_id = teams[0].as_str().unwrap().parse::<uuid::Uuid>().unwrap();
        let rows = sqlx::query("SELECT actor_user_id,event_type,resource_id,created_at::text AS created_at,metadata FROM latex_core.audit_events WHERE event_type IN ('review.round.opened','review.feedback.published') AND metadata->>'paper_id'=$1")
            .bind(paper_id.to_string()).fetch_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 2, "one durable Leader event and one Mentor event despite rejected and repeated publication requests");
        let mut round_id = None;
        for row in rows {
            let event: String = row.try_get("event_type").unwrap();
            let actor: uuid::Uuid = row.try_get("actor_user_id").unwrap();
            assert_eq!(actor, *if event == "review.round.opened" { writer_id.as_uuid() } else { mentor_id.as_uuid() });
            let resource: uuid::Uuid = row.try_get("resource_id").unwrap();
            if let Some(expected) = round_id { assert_eq!(resource, expected); } else { round_id = Some(resource); }
            let timestamp: String = row.try_get("created_at").unwrap();
            assert!(!timestamp.is_empty());
            let metadata: serde_json::Value = row.try_get("metadata").unwrap();
            assert_eq!(metadata["paper_id"], paper_id.to_string());
            assert_eq!(metadata["outcome"], "SUCCESS");
            if event == "review.feedback.published" { assert_eq!(metadata["published_count"], 2); }
            println!("DURABLE_REVIEW_AUDIT_PASS: event={event} actor={actor} paper={paper_id} round={resource} outcome=SUCCESS");
        }
    }

    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn team_grouped_credentials_use_only_materialized_import_memberships() {
    let _guard = SERVER_TEST_LOCK.lock().await;
    let (database, pool, app, _storage, state) = test_application().await;
    let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
    let reused = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    fixture_default_template(&state, &pool, &admin).await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let email = |name: &str| format!("{name}-{suffix}@example.invalid");
    let files = vec![
        ("students.csv", format!("reg_no,name,email\nA{suffix},Leader A,{}\nB{suffix},Leader B,{}\nC{suffix},Leader C,{}\nU{suffix},Unassigned,{}\nR{suffix},Reused,{}\n",email("a"),email("b"),email("c"),email("u"),reused.email).into_bytes()),
        ("faculty.csv",format!("faculty_id,name,email\nF{suffix},Shared Mentor,{}\n",email("m")).into_bytes()),
        ("paper_teams.csv",format!("external_team_key,team_name,academic_year,semester,status\nC{suffix},Zulu,2026,1,ACTIVE\nB{suffix},Alpha,2026,1,ACTIVE\nA{suffix},Alpha,2026,1,ACTIVE\n").into_bytes()),
        ("paper_team_writers.csv",format!("external_team_key,student_reg_no,writer_order,is_leader\nC{suffix},C{suffix},1,true\nB{suffix},B{suffix},1,true\nA{suffix},A{suffix},1,true\nA{suffix},R{suffix},2,false\n").into_bytes()),
        ("paper_team_mentors.csv",format!("external_team_key,faculty_id\nA{suffix},F{suffix}\nB{suffix},F{suffix}\n").into_bytes()),
    ];
    let (body, content_type) = institution_batch_multipart(&files);
    let validated = test_json(
        request_bytes(
            &app,
            Method::POST,
            "/api/admin/v2/institution/import-batches/validate",
            Some(&admin.cookie),
            body,
            Some(&content_type),
        )
        .await,
    )
    .await;
    assert_eq!(
        validated["batch"]["status"], "VALIDATED",
        "synthetic batch must validate"
    );
    let path = format!(
        "/api/admin/v2/institution/import-batches/{}/apply",
        validated["batch"]["id"].as_str().unwrap()
    );
    assert_eq!(
        request(
            &app,
            Method::POST,
            &path,
            Some(&reused.cookie),
            "{}",
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let applied = test_json(
        request(
            &app,
            Method::POST,
            &path,
            Some(&admin.cookie),
            "{}",
            Some("application/json"),
        )
        .await,
    )
    .await;
    // A shared Mentor must not pull unrelated Team metadata into this batch.
    let shared_mentor = test_user_id(&pool, &email("m")).await;
    let reused_writer = test_user_id(&pool, &reused.email).await;
    let outside = request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
        &serde_json::json!({"name":"Outside this import","writer_ids":[reused_writer],"leader_writer_id":reused_writer,"mentor_ids":[shared_mentor]}).to_string(),Some("application/json")).await;
    assert_eq!(outside.status(), StatusCode::CREATED);
    let batch_id = uuid::Uuid::parse_str(validated["batch"]["id"].as_str().unwrap()).unwrap();
    let job_id = state.institution.batch_primary_job(batch_id).await.unwrap();
    let mut grouping: persistence::AccountProvisioningResult =
        serde_json::from_value(applied["account_provisioning"].clone()).unwrap();
    state
        .institution
        .group_generated_credentials(job_id, &mut grouping)
        .await
        .unwrap();
    let mentor_groups = &grouping
        .credentials
        .iter()
        .find(|item| item.email == email("m"))
        .unwrap()
        .teams;
    assert_eq!(mentor_groups.len(), 2);
    assert!(
        mentor_groups
            .iter()
            .all(|team| team.team_name != "Outside this import")
    );
    let credentials = applied["account_provisioning"]["credentials"]
        .as_array()
        .unwrap();
    assert_eq!(credentials.len(), 5);
    assert!(!credentials.iter().any(|item| item["email"] == reused.email));
    for name in ["a", "b", "c", "u", "m"] {
        let item = credentials
            .iter()
            .find(|item| item["email"] == email(name))
            .unwrap();
        let teams = item["teams"].as_array().unwrap();
        assert_eq!(
            teams.len(),
            match name {
                "m" => 2,
                "u" => 0,
                _ => 1,
            }
        );
        for team in teams {
            assert!(team["team_id"].as_str().is_some());
            assert_eq!(
                team["team_role"],
                if name == "m" { "mentor" } else { "writer" }
            );
            assert_eq!(team["is_leader"], name != "m");
        }
        let password = item["temporary_password"].as_str().unwrap();
        let login = login_request(&app, &email(name), password).await;
        assert_eq!(login.headers()[header::LOCATION], "/change-password");
    }
    let retry = test_json(
        request(
            &app,
            Method::POST,
            &path,
            Some(&admin.cookie),
            "{}",
            Some("application/json"),
        )
        .await,
    )
    .await;
    assert!(
        retry["account_provisioning"]["credentials"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    println!(
        "TEAM_GROUPED_CREDENTIALS_PASS: three imported Teams, duplicate names, shared Mentor, unassigned and reused account; passwords authenticate and force change; retry reveals none"
    );
    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn captured_report_sources_do_not_mix_live_workspace_generations() {
    let _guard = SERVER_TEST_LOCK.lock().await;
    let (database, pool, app, _storage, state) = test_application().await;
    let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let owner = test_user_id(&pool, &writer.email).await;
    let tenant: uuid::Uuid =
        sqlx::query_scalar("SELECT tenant_id FROM latex_core.users WHERE id=$1")
            .bind(owner.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let workspace = WorkspaceId::new();
    let main = LogicalPath::parse("main.tex").unwrap();
    let chapter = LogicalPath::parse("chapter.tex").unwrap();
    let initial = state
        .workspaces
        .create_workspace(
            core_types::TenantId::from_uuid(tenant),
            owner,
            workspace,
            main,
            Bytes::from_static(b"\\input{chapter}\n"),
        )
        .await
        .unwrap();
    state
        .workspaces
        .put_file(
            workspace,
            owner,
            initial.version(),
            chapter.clone(),
            Bytes::from_static(b"\\newcommand{\\thesistitle}{Original}\n"),
        )
        .await
        .unwrap();
    let captured = state.workspaces.restore(workspace).await.unwrap();
    state
        .workspaces
        .put_file(
            workspace,
            owner,
            captured.version(),
            chapter.clone(),
            Bytes::from_static(b"\\newcommand{\\thesistitle}{Updated}\n"),
        )
        .await
        .unwrap();
    let original = captured_report_source(
        &state,
        workspace,
        &chapter,
        captured.file(&chapter).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        original.as_ref(),
        b"\\newcommand{\\thesistitle}{Original}\n"
    );
    assert_eq!(
        state
            .workspaces
            .read_file(workspace, &chapter)
            .await
            .unwrap()
            .as_ref(),
        b"\\newcommand{\\thesistitle}{Updated}\n"
    );
    pool.close().await;
    database.close().await;
}
