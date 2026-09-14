// Included inside database_tests to reuse authenticated, disposable fixtures.
#[tokio::test]
async fn legacy_front_matter_institutional_api_contract() {
    let _guard = SERVER_TEST_LOCK.lock().await;
    let (database, pool, mut app, _storage, mut state) = test_application().await;
    let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
    let admin_id = test_user_id(&pool, &admin.email).await;
    let mut writers = Vec::new();
    let mut writer_ids = Vec::new();
    let suffix = uuid::Uuid::new_v4().to_string();
    let programme = format!("FM-{suffix}");
    let hod = format!("HOD-{suffix}");
    let dean = format!("DEAN-{suffix}");
    sqlx::query("INSERT INTO vcap.faculty (faculty_id,name,honorific) VALUES ($1,'Helen Head','Dr.'),($2,'Dana Dean','Prof.')")
        .bind(&hod).bind(&dean).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.programmes (programme_code,hod_id) VALUES ($1,$2)")
        .bind(&programme)
        .bind(&hod)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO vcap.faculty_roles (role_id,faculty_id,role_type,programme_code,status) VALUES ($1,$2,'Dean',$3,'ACTIVE')").bind(uuid::Uuid::new_v4()).bind(&dean).bind(&programme).execute(&pool).await.unwrap();
    let school = format!("SCHOOL-{suffix}");
    sqlx::query("INSERT INTO vcap.schools (school_id) VALUES ($1)")
        .bind(&school)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO vcap.faculty_roles (role_id,faculty_id,role_type,programme_code,school_id,status) VALUES ($1,$2,'HOD',$3,$4,'ACTIVE')").bind(uuid::Uuid::new_v4()).bind(&hod).bind(&programme).bind(&school).execute(&pool).await.unwrap();
    sqlx::query("UPDATE vcap.faculty_roles SET programme_code=NULL,school_id=$2 WHERE faculty_id=$1 AND role_type='Dean'").bind(&dean).bind(&school).execute(&pool).await.unwrap();
    for (index, name) in [
        "Alice Alpha",
        "Ben Beta",
        "Cara Gamma",
        "Dev Delta",
        "Eva Extra",
    ]
    .iter()
    .enumerate()
    {
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let id = test_user_id(&pool, &writer.email).await;
        let reg = format!("FM{}-{suffix}", index + 1);
        sqlx::query("INSERT INTO vcap.students (reg_no,name,programme_code) VALUES ($1,$2,$3)")
            .bind(&reg)
            .bind(name)
            .bind(&programme)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO vcap.student_user_links (reg_no,user_id,status,linked_at) VALUES ($1,$2,'LINKED',now())").bind(&reg).bind(id.as_uuid()).execute(&pool).await.unwrap();
        writers.push(writer);
        writer_ids.push(id);
    }
    let mut mentors = Vec::new();
    let mut mentor_ids = Vec::new();
    for (index, name) in ["Grace Guide", "Morgan Mentor"].iter().enumerate() {
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let id = test_user_id(&pool, &mentor.email).await;
        let faculty = format!("GUIDE{index}-{suffix}");
        sqlx::query("INSERT INTO vcap.faculty (faculty_id,name,honorific,designation) VALUES ($1,$2,'Dr.','Associate Professor')").bind(&faculty).bind(name).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO vcap.faculty_user_links (faculty_id,user_id,status,linked_at) VALUES ($1,$2,'LINKED',now())").bind(&faculty).bind(id.as_uuid()).execute(&pool).await.unwrap();
        mentors.push(mentor);
        mentor_ids.push(id);
    }
    let template_id = uuid::Uuid::new_v4();
    let template = state.blobs.put(Bytes::from_static(b"\\documentclass{article}\n\\usepackage{graphicx}\n\\usepackage{listings}\n\\newcommand{\\studentAname}{Student A name}\n\\newcommand{\\projguidename}{Dr. Project guide name}\n\\newcommand{\\hoddept}{departmentname}\n\\begin{document}\n\\input{../.latex-core/frontmatter/frontmatter.tex} % LATEX_CORE_FRONT_MATTER\nMain content target phrase.\nInlineSlot\\par\nSymbolSlot\\par\nCodeSlot\nTableSlot\nFigureSlot\n\\input{chapters/chapter1.tex}\n\\begin{thebibliography}{9}\n\\bibitem{placeholder} PublicationSlot\n\\end{thebibliography}\n\\end{document}\n")).await.unwrap();
    let chapter = state.blobs.put(Bytes::from_static(b"Existing chapter.\n")).await.unwrap();
    let image_directory = state.blobs.put(Bytes::from_static(b"Project image directory.\n")).await.unwrap();
    state
        .repo
        .create_template_with_compatibility(
            template_id,
            &format!("Legacy test {suffix}"),
            None,
            Some("Thesis_content_v1.0/main.tex"),
            true,
            &[
                AppTemplateFileRecord {
                    path: "Thesis_content_v1.0/main.tex".into(),
                    blob_hash: template.hash(),
                    size_bytes: template.size_bytes(),
                },
                AppTemplateFileRecord {
                    path: "Thesis_content_v1.0/chapters/chapter1.tex".into(),
                    blob_hash: chapter.hash(),
                    size_bytes: chapter.size_bytes(),
                },
                AppTemplateFileRecord {
                    path: "Thesis_content_v1.0/images/README.txt".into(),
                    blob_hash: image_directory.hash(),
                    size_bytes: image_directory.size_bytes(),
                },
            ],
        )
        .await
        .unwrap();
    let archive = test_zip(&[
        (
            "coverpage.tex",
            include_bytes!("../tests/fixtures/vit-front-matter/coverpage.tex"),
        ),
        (
            "certificate.tex",
            include_bytes!("../tests/fixtures/vit-front-matter/certificate.tex"),
        ),
        (
            "declaration.tex",
            include_bytes!("../tests/fixtures/vit-front-matter/declaration.tex"),
        ),
        (
            "acknowledgement.tex",
            include_bytes!("../tests/fixtures/vit-front-matter/acknowledgement.tex"),
        ),
    ]);
    let (body, content_type) =
        template_multipart(&[("name", &format!("Synthetic VIT {suffix}"))], &archive);
    let response = request_bytes(
        &app,
        Method::POST,
        "/api/admin/v2/front-matter-packs/import",
        Some(&admin.cookie),
        body,
        Some(&content_type),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let imported = test_json(response).await;
    let pack_id = imported["id"].as_str().unwrap();
    let mut representative = None;
    for size in (1..=5).filter(|size| env::var_os("FRONTMATTER_BROWSER").is_none() || *size == 4) {
        let response = request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie), &serde_json::json!({"name":"Synthetic Solar Project", "writer_ids":writer_ids[..size], "leader_writer_id":writer_ids[0], "mentor_ids":[mentor_ids[0]], "template_id":template_id,"front_matter_pack_id":pack_id}).to_string(), Some("application/json")).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let created = test_json(response).await;
        let paper_id = uuid::Uuid::parse_str(created["team"]["id"].as_str().unwrap()).unwrap();
        let import_job = uuid::Uuid::new_v4();
        let external_key = format!("FM-{paper_id}");
        sqlx::query("INSERT INTO latex_core.institution_import_jobs (id,import_kind,mode,original_filename,content_sha256,file_type,submitted_by_user_id) VALUES ($1,'paper_teams','MERGE','synthetic.csv',$2,'CSV',$3)").bind(import_job).bind("a".repeat(64)).bind(admin_id.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO vcap.paper_assignment_groups (external_team_key,team_name,semester) VALUES ($1,'Synthetic Solar Project','VIII')").bind(&external_key).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO latex_core.external_paper_team_links (external_team_key,paper_team_id,source_import_job_id) VALUES ($1,$2,$3)").bind(&external_key).bind(paper_id).bind(import_job).execute(&pool).await.unwrap();
        let path = format!("/api/v2/papers/{paper_id}/document-details");
        let detail = test_json(get(&app, &path, Some(&writers[0].cookie)).await).await;
        let values = detail["values"].as_array().unwrap();
        let value = |key: &str| {
            values
                .iter()
                .find(|value| value["field_key"] == key)
                .unwrap()["value"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(value("team_size"), size.to_string());
        assert_eq!(value("team_semester"), "VIII");
        assert_eq!(value("student_a_name"), "Alice Alpha");
        assert_eq!(value("guide_name"), "Dr. Grace Guide");
        assert_eq!(value("guide_designation"), "Associate Professor");
        assert_eq!(value("hod_name"), "Dr. Helen Head");
        assert_eq!(value("dean_name"), "Prof. Dana Dean");
        assert!(
            detail["missing_required_fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item == "Department name")
        );
        assert_eq!(
            detail["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item.as_str().unwrap().contains("additional Writers")),
            size > 4
        );
        assert!(!detail.to_string().contains("gender"));
        if size == 4 {
            representative = Some((paper_id, path, created));
        }
    }
    let (paper_id, path, created) = representative.unwrap();
    let project_path = format!("/api/v2/papers/{paper_id}/project-metadata");
    let project_before = test_json(get(&app, &project_path, Some(&writers[0].cookie)).await).await;
    assert_eq!(project_before["setup_complete"], false);
    let jobs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.compile_jobs WHERE workspace_id=$1")
        .bind(uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap()).fetch_one(&pool).await.unwrap();
    let project_input = serde_json::json!({
        "executive_summary":"A literal executive summary, distinct from an abstract.",
        "project_type":"capstone",
        "datasets":[{"name":"Synthetic Solar Set","url":"https://example.invalid/data","description":"Synthetic browser fixture"}],
        "source_code_snippets":[{"label":"Literal Rust","language":"Rust","code":"fn main() {\n    println!(\"# % & _\");\n}"}],
        "publications":[
            {"citation_key":"solar-communicated","authors":"A. Alpha","title":"Draft Solar Work","venue":"Example Venue","year":2026,"doi":null,"url":null,"status":"communicated"},
            {"citation_key":"solar-accepted","authors":"B. Beta","title":"Accepted Solar Work","venue":"Example Venue","year":2026,"doi":null,"url":null,"status":"accepted"},
            {"citation_key":"solar-published","authors":"C. Gamma","title":"Published Solar Work","venue":"Example Venue","year":2026,"doi":"10.1/example","url":null,"status":"published"}
        ],
        "setup_complete":true
    });
    assert_eq!(request(&app, Method::PUT, &project_path, Some(&writers[0].cookie), &project_input.to_string(), Some("application/json")).await.status(), StatusCode::OK);
    let project_after = test_json(get(&app, &project_path, Some(&writers[0].cookie)).await).await;
    assert_eq!(project_after["values"]["project_type"], "capstone");
    assert_eq!(project_after["values"]["source_code_snippets"][0]["code"], "fn main() {\n    println!(\"# % & _\");\n}");
    assert_eq!(project_after["values"]["publications"].as_array().unwrap().len(), 3);
    let jobs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.compile_jobs WHERE workspace_id=$1")
        .bind(uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!(jobs_before, jobs_after, "project metadata saves must not compile");
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM latex_core.paper_versions WHERE paper_id=$1")
            .bind(paper_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    for _ in 0..2 {
        assert_eq!(
            get(&app, &path, Some(&writers[0].cookie)).await.status(),
            StatusCode::OK
        );
    }
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM latex_core.paper_versions WHERE paper_id=$1")
            .bind(paper_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after, "AUTO reads must not create versions");
    let manual = serde_json::json!({"course_code":"CSE4999","course_name":"Capstone Project","degree_name":"Bachelor of Technology","programme_name":"Computer Science and Engineering","specialization":"Intelligent Systems","submission_date":"2026-09-13","school_name":"School of Computing","department_name":"Computer Science"});
    let body = serde_json::json!({"values":manual,"sections":{}}).to_string();
    for cookie in [&writers[1].cookie, &mentors[0].cookie] {
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &path,
                Some(cookie),
                &body,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
    let outsider = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    assert_eq!(
        request(
            &app,
            Method::PUT,
            &path,
            Some(&outsider.cookie),
            &body,
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    for values in [
        serde_json::json!({"team_name":"Invented title"}),
        serde_json::json!({"student_a_name":"Fake student"}),
        serde_json::json!({"guide_identity":admin_id}),
        serde_json::json!({"guide_identity":123}),
        serde_json::json!({"submission_date":"2026-02-30"}),
    ] {
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &path,
                Some(&writers[0].cookie),
                &serde_json::json!({"values":values,"sections":{}}).to_string(),
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let save = request(
        &app,
        Method::PUT,
        &path,
        Some(&writers[0].cookie),
        &body,
        Some("application/json"),
    )
    .await;
    let save_status = save.status();
    let save_body = test_text(save).await;
    assert_eq!(save_status, StatusCode::OK, "{save_body}");
    let detail = test_json(get(&app, &path, Some(&writers[0].cookie)).await).await;
    assert_eq!(detail["missing_required_fields"], serde_json::json!([]));
    let versions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM latex_core.paper_versions WHERE paper_id=$1")
            .bind(paper_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(versions, before + 1);
    let workspace = WorkspaceId::from_uuid(
        uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap(),
    );
    let metadata_path = LogicalPath::parse(".latex-core/frontmatter/metadata.tex").unwrap();
    let saved_metadata = state
        .workspaces
        .read_file(workspace, &metadata_path)
        .await
        .unwrap();
    let semantic_path = LogicalPath::parse(".latex-core/frontmatter/Front-Matter.tex").unwrap();
    let semantic = state.workspaces.read_file(workspace, &semantic_path).await.unwrap();
    let semantic_text = String::from_utf8(semantic.to_vec()).unwrap();
    assert!(semantic_text.contains("canonical: course_code"));
    assert!(semantic_text.contains("LatexCoreCourseCode"));
    let hidden: uuid::Uuid = sqlx::query_scalar("SELECT f.file_id FROM latex_core.paper_files f JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id WHERE f.workspace_id=$1 AND f.path='.latex-core/frontmatter/metadata.tex' AND p.policy='HIDDEN_SYSTEM' AND NOT f.tombstoned").bind(workspace.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        get(
            &app,
            &format!("/api/v2/papers/{paper_id}/files/{hidden}"),
            Some(&writers[0].cookie)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let stored_sources: Vec<String> = sqlx::query_scalar("SELECT DISTINCT value_source FROM latex_core.paper_front_matter_values WHERE paper_team_id=$1").bind(paper_id).fetch_all(&pool).await.unwrap();
    assert_eq!(stored_sources, vec!["TEAM_OVERRIDE"]);
    sqlx::query("INSERT INTO latex_core.paper_team_members (paper_team_id,user_id,is_leader,assigned_by_user_id) VALUES ($1,$2,false,$3)").bind(paper_id).bind(mentor_ids[1].as_uuid()).bind(admin_id.as_uuid()).execute(&pool).await.unwrap();
    let detail = test_json(get(&app, &path, Some(&writers[0].cookie)).await).await;
    let guide_field = detail["manifest"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["key"] == "guide_identity")
        .unwrap();
    assert_eq!(guide_field["options"].as_array().unwrap().len(), 2);
    assert!(
        detail["missing_required_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "Project guide selection")
    );
    let mut selected = manual.clone();
    selected["guide_identity"] = serde_json::json!(mentor_ids[1]);
    assert_eq!(
        request(
            &app,
            Method::PUT,
            &path,
            Some(&writers[0].cookie),
            &serde_json::json!({"values":selected,"sections":{}}).to_string(),
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    let detail = test_json(get(&app, &path, Some(&writers[0].cookie)).await).await;
    assert!(
        detail["values"].as_array().unwrap().iter().any(
            |value| value["field_key"] == "guide_name" && value["value"] == "Dr. Morgan Mentor"
        )
    );
    let version_a: uuid::Uuid = sqlx::query_scalar("SELECT id FROM latex_core.paper_versions WHERE paper_id=$1 AND version_type='front_matter_update' ORDER BY version_number DESC LIMIT 1").bind(paper_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        request(
            &app,
            Method::POST,
            &format!("/api/v2/papers/{paper_id}/versions/{version_a}/revert"),
            Some(&writers[0].cookie),
            r#"{"confirmed":true}"#,
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        state
            .workspaces
            .read_file(workspace, &metadata_path)
            .await
            .unwrap(),
        saved_metadata,
        "restore must retain exact generated metadata bytes"
    );
    sqlx::query("UPDATE latex_core.paper_teams SET name=$2 WHERE id=$1")
        .bind(paper_id)
        .bind(r"Title & % $ # _ { } \input \write18")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(
            &app,
            Method::PUT,
            &path,
            Some(&writers[0].cookie),
            &serde_json::json!({"values":selected,"sections":{}}).to_string(),
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    let malicious = state
        .workspaces
        .read_file(workspace, &metadata_path)
        .await
        .unwrap();
    let escaped = String::from_utf8(malicious.to_vec()).unwrap();
    assert!(!escaped.contains(r"\write18"));
    assert!(escaped.contains(r"\textbackslash{}write18"));
    sqlx::query("UPDATE latex_core.paper_teams SET name='Synthetic Solar Project' WHERE id=$1")
        .bind(paper_id)
        .execute(&pool)
        .await
        .unwrap();
    // Optional opt-in uses the real frozen compiler and one authenticated browser journey.
    if env::var_os("FRONTMATTER_BROWSER").is_some() {
        legacy_front_matter_browser(
            &mut state, &mut app, &pool, &admin, &writers, &mentors, paper_id, &created, &manual,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn legacy_front_matter_browser(
    state: &mut AppState,
    app: &mut Router,
    pool: &PgPool,
    admin: &Fixture,
    writers: &[Fixture],
    mentors: &[Fixture],
    paper_id: uuid::Uuid,
    created: &serde_json::Value,
    manual: &serde_json::Value,
) {
    let staging = tempfile::tempdir().unwrap();
    let runtime = compiler::DockerCliRuntime::new("latex-core-texlive@sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38").unwrap();
    let compiler = compiler::CompilerService::new(
        state.blobs.clone(),
        runtime,
        compiler::CompilerConfig::new(compiler::CompileLimits::development_default())
            .with_staging_root(staging.path().to_path_buf()),
    )
    .unwrap();
    state.environment = compiler.environment_id().clone();
    // Remove only the fixture's saved manual values so this journey exercises first use.
    sqlx::query("DELETE FROM latex_core.paper_front_matter_values WHERE paper_team_id=$1")
        .bind(paper_id)
        .execute(pool)
        .await
        .unwrap();
    *app = router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, served).await.unwrap();
    });
    let worker = queue::CompilationWorker::new(
        state.queue.clone(),
        state.blobs.clone(),
        Arc::new(compiler),
        core_types::WorkerId::new(),
        queue::WorkerConfig::new(1, Duration::from_millis(100)).unwrap(),
    );
    let shutdown = queue::WorkerShutdown::new();
    let worker_shutdown = shutdown.clone();
    let worker_task = tokio::spawn(async move {
        worker.run_until_shutdown(worker_shutdown).await.unwrap();
    });
    let config = serde_json::json!({"base":format!("http://{address}"),"paper_id":paper_id,"paper_name":created["team"]["name"],"leader":writers[0].email,"writer":writers[1].email,"mentor":mentors[0].email,"mentor_two":mentors[1].email,"password":PASSWORD,"manual":manual,"environment":state.environment.to_string()});
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("node")
            .arg("tests/front-matter-browser.mjs")
            .env("FRONTMATTER_TEST_CONFIG", config.to_string())
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "browser stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));

    let integration = test_json(
        request(
            app,
            Method::POST,
            "/api/admin/integration/v1/clients",
            Some(&admin.cookie),
            &serde_json::json!({
                "name":format!("browser archive {}",uuid::Uuid::new_v4()),
                "scopes":["reports.read","reports.files.read","reports.pdf.read"],
                "institution_wide":false,
                "report_ids":[paper_id],
                "expires_at":null
            })
            .to_string(),
            Some("application/json"),
        )
        .await,
    )
    .await;
    let secret = integration["secret"].as_str().unwrap().to_owned();
    let archive = tempfile::tempdir().unwrap();
    let archive_path = archive.path().to_owned();
    let client_output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("python3")
            .arg("../../examples/institutional-archive/archive.py")
            .arg("--base-url")
            .arg(format!("http://{address}"))
            .arg("--output")
            .arg(&archive_path)
            .env("LATEX_CORE_INTEGRATION_TOKEN", secret)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        client_output.status.success(),
        "archive client stderr: {}",
        String::from_utf8_lossy(&client_output.stderr)
    );
    let report_root = archive.path().join("reports").join(paper_id.to_string());
    assert!(report_root.join("front-matter.json").is_file());
    assert!(
        std::fs::read_dir(report_root.join("pdf"))
            .unwrap()
            .any(|entry| entry.unwrap().path().extension().is_some_and(|value| value == "pdf")),
        "archive client did not download the existing PDF"
    );
    println!("archive client: passed (metadata, immutable files, existing PDF hash)");
    shutdown.request();
    worker_task.await.unwrap();
    server.abort();
}
