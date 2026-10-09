// Runs only against the host script's disposable database and existing frozen
// compiler image. Compilation and PDF text checks are mandatory in this test.
async fn fixture_default_template(state: &AppState, pool: &PgPool, admin: &Fixture) {
    let admin_id = test_user_id(pool, &admin.email).await;
    let template_id = uuid::Uuid::new_v4();
    let source = state
        .blobs
        .put(Bytes::from_static(
            b"\\documentclass{article}\n\\begin{document}Disposable fixture\\end{document}\n",
        ))
        .await
        .unwrap();
    state
        .repo
        .create_template(
            template_id,
            &format!("Disposable fixture default {template_id}"),
            None,
            Some("main.tex"),
            &[AppTemplateFileRecord {
                path: "main.tex".into(),
                blob_hash: source.hash(),
                size_bytes: source.size_bytes(),
            }],
        )
        .await
        .unwrap();
    state
        .institution
        .set_global_fallback(admin_id, template_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn course_autofill_persists_complete_report_and_renders_real_pdf() {
    let _guard = SERVER_TEST_LOCK.lock().await;
    let (database, pool, _app, _storage, mut state) = test_application().await;
    // No email delivery or outbox enqueueing for the uploaded student addresses.
    state.institution = InstitutionRepository::new(database.clone());
    state.repo = AppRepository::new(database.clone());
    state.mail_outbox = None;
    state.mail_enabled = false;
    let image = env::var("LATEX_CORE_COURSE_TEXLIVE_IMAGE")
        .expect("host script must supply the existing frozen TeX Live image");
    let staging = tempfile::tempdir().unwrap();
    let compiler = Arc::new(
        compiler::CompilerService::new(
            state.blobs.clone(),
            compiler::DockerCliRuntime::new(image.clone()).unwrap(),
            compiler::CompilerConfig::new(compiler::CompileLimits::development_default())
                .with_staging_root(staging.path().to_path_buf()),
        )
        .unwrap(),
    );
    state.environment = compiler.environment_id().clone();
    let app = router(state.clone());
    let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
    let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let other = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let leader_b = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let leader_c = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let leader_b_id = test_user_id(&pool, &leader_b.email).await;
    let leader_c_id = test_user_id(&pool, &leader_c.email).await;
    let writer_id = test_user_id(&pool, &writer.email).await;
    let other_id = test_user_id(&pool, &other.email).await;
    let workbook = std::fs::read(
        env::var("LATEX_CORE_ACCEPTANCE_WORKBOOK")
            .expect("host runner must provide the unchanged Project-Stage-1 workbook"),
    )
    .unwrap();
    let (body, content_type) = institution_batch_multipart(&[("Project-Stage-1.xlsx", workbook)]);
    let response = request_bytes(
        &app,
        Method::POST,
        "/api/admin/v2/institution/import-batches/validate",
        Some(&admin.cookie),
        body,
        Some(&content_type),
    )
    .await;
    let status = response.status();
    let validated = test_json(response).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "workbook validation: {validated}"
    );
    assert_eq!(validated["batch"]["status"], "VALIDATED", "{validated}");
    let batch = validated["batch"]["id"]
        .as_str()
        .expect("successful import has a batch ID");
    let response = request(
        &app,
        Method::POST,
        &format!("/api/admin/v2/institution/import-batches/{batch}/apply"),
        Some(&admin.cookie),
        "{}",
        Some("application/json"),
    )
    .await;
    let status = response.status();
    let applied = test_json(response).await;
    assert_eq!(status, StatusCode::OK, "workbook apply: {applied}");
    assert_eq!(applied["batch"]["status"], "APPLIED", "{applied}");
    let uploaded: Vec<(String,String,String,String)> = sqlx::query_as("SELECT student_reg_no,course_id,academic_year,semester FROM vcap.student_course_registrations WHERE course_id IN ('BCSE497J','MACSE698') ORDER BY student_reg_no")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(uploaded.len(), 48);
    assert_eq!(
        uploaded.iter().filter(|row| row.1 == "BCSE497J").count(),
        27
    );
    assert_eq!(
        uploaded.iter().filter(|row| row.1 == "MACSE698").count(),
        21
    );
    assert!(
        uploaded
            .iter()
            .all(|row| row.2 == "2026-2027" && row.3 == "FALL")
    );
    let undergraduate = uploaded
        .iter()
        .find(|row| row.1 == "BCSE497J")
        .unwrap()
        .0
        .clone();
    let postgraduate = uploaded
        .iter()
        .find(|row| row.1 == "MACSE698")
        .unwrap()
        .0
        .clone();
    // The workbook has no Team/Leader assignments. These assignments are synthetic.
    let nonleader_registration = format!("SYNTHETIC-NONLEADER-{}", uuid::Uuid::new_v4());
    let synthetic_registration = format!("SYNTHETIC-BCSE4973-{}", uuid::Uuid::new_v4());
    for (reg, user, code, year, semester) in [
        (
            &nonleader_registration,
            other_id,
            "NONLEADER",
            "2029-2030",
            "WINTER",
        ),
        (
            &synthetic_registration,
            writer_id,
            "BCSE4973",
            "2026-2027",
            "FALL",
        ),
    ] {
        sqlx::query("INSERT INTO vcap.students(reg_no,name) VALUES($1,'Explicitly synthetic acceptance student')").bind(reg).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO vcap.student_course_registrations(student_reg_no,course_id,academic_year,semester,registration_status) VALUES($1,$2,$3,$4,'REGISTERED')")
            .bind(reg).bind(code).bind(year).bind(semester).execute(&pool).await.unwrap();
        if user == other_id {
            sqlx::query("INSERT INTO vcap.student_user_links(reg_no,user_id,status,linked_at) VALUES($1,$2,'LINKED',now())").bind(reg).bind(user.as_uuid()).execute(&pool).await.unwrap();
        }
    }
    sqlx::query("UPDATE vcap.student_user_links SET user_id=$2,status='LINKED',linked_at=now() WHERE reg_no=$1")
        .bind(&undergraduate).bind(writer_id.as_uuid()).execute(&pool).await.unwrap();
    for (registration, leader) in [
        (&postgraduate, leader_b_id),
        (&synthetic_registration, leader_c_id),
    ] {
        sqlx::query("INSERT INTO vcap.student_user_links(reg_no,user_id,status,linked_at) VALUES($1,$2,'LINKED',now()) ON CONFLICT(reg_no) DO UPDATE SET user_id=EXCLUDED.user_id,status='LINKED',linked_at=now()")
            .bind(registration).bind(leader.as_uuid()).execute(&pool).await.unwrap();
    }
    let main_name = "Full_Report_template_v1.0/Full_Report_v1.0.tex";
    let mut original = zip::ZipArchive::new(Cursor::new(include_bytes!(
        "../../../artifacts/Full_Report_template_v1.1.zip"
    )))
    .unwrap();
    let mut entries = Vec::new();
    for index in 0..original.len() {
        let mut file = original.by_index(index).unwrap();
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        entries.push((name, bytes));
    }
    let archive = test_zip(
        &entries
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>(),
    );
    let (body, content_type) = template_multipart(
        &[
            ("name", "Disposable Complete Report course acceptance"),
            ("main", main_name),
            ("arrangement", "SINGLE_SOURCE"),
        ],
        &archive,
    );
    let response = request_bytes(
        &app,
        Method::POST,
        "/api/admin/v2/templates/import",
        Some(&admin.cookie),
        body,
        Some(&content_type),
    )
    .await;
    let status = response.status();
    let template = test_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "{template}");
    // Create every Team, pin its template and seed historical overrides BEFORE
    // exercising Details. Each Team keeps its own assigned Leader/registration;
    // no recreation, membership changes, reimport or migration repairs the data.
    let mut teams = Vec::new();
    for (name, leader) in [
        ("Existing Team A", writer_id),
        ("Existing Team B", leader_b_id),
        ("Synthetic BCSE4973 Team", leader_c_id),
    ] {
        let response = request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
            &serde_json::json!({"name":name,"template_id":template["id"],"writer_ids":[leader,other_id],"leader_writer_id":leader,"use_front_matter_default":false}).to_string(), Some("application/json")).await;
        let status = response.status();
        let created = test_json(response).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = uuid::Uuid::parse_str(created["team"]["id"].as_str().unwrap()).unwrap();
        for (key, value) in [
            ("course_code", "BA101CSE101"),
            ("team_academic_year", "2025-2026"),
            ("team_semester", "Winter Semester"),
        ] {
            sqlx::query("INSERT INTO latex_core.paper_front_matter_values(paper_team_id,field_key,value_json,value_source,updated_by_user_id) VALUES($1,$2,$3,'TEAM_OVERRIDE',$4) ON CONFLICT(paper_team_id,field_key) DO UPDATE SET value_json=EXCLUDED.value_json")
                .bind(id).bind(key).bind(serde_json::json!(value)).bind(leader.as_uuid()).execute(&pool).await.unwrap();
        }
        teams.push(id);
    }
    let paper_id = teams[0];
    let paper = state.v2.writer_paper(writer_id, paper_id).await.unwrap();
    let main_path = LogicalPath::parse(main_name).unwrap();
    let detail_path = format!("/api/v2/papers/{paper_id}/document-details");
    let initial_source = state
        .workspaces
        .read_file(paper.workspace_id, &main_path)
        .await
        .unwrap();
    let detail = test_json(get(&app, &detail_path, Some(&writer.cookie)).await).await;
    for (key, value) in [
        ("course_code", "BCSE497J"),
        ("course_name", "Project-I"),
        ("team_semester", "Fall Semester"),
        ("team_academic_year", "2026-2027"),
    ] {
        assert!(
            detail["values"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["field_key"] == key && item["value"] == value),
            "{detail}"
        );
    }
    assert_eq!(
        state
            .workspaces
            .read_file(paper.workspace_id, &main_path)
            .await
            .unwrap(),
        initial_source,
        "opening Details must not rewrite source"
    );
    let nonleader = test_json(get(&app, &detail_path, Some(&other.cookie)).await).await;
    assert_eq!(nonleader["can_edit"], false);
    for key in ["course_code", "team_academic_year", "team_semester"] {
        assert_eq!(
            nonleader["values"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["field_key"] == key)
                .unwrap()["value"],
            detail["values"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["field_key"] == key)
                .unwrap()["value"]
        );
        assert!(
            !detail["manifest"]["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["key"] == key)
                .unwrap()["allow_team_override"]
                .as_bool()
                .unwrap()
        );
        let response = request(
            &app,
            Method::PUT,
            &detail_path,
            Some(&writer.cookie),
            &serde_json::json!({"values":{key:"forged"},"sections":{}}).to_string(),
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let editable = detail["manifest"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|field| field["allow_team_override"] == true)
        .map(|field| field["key"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        editable,
        BTreeSet::from(["course_name", "project_title", "submission_date"])
    );
    for (code, expected, paper_id, writer, writer_id) in [
        ("BCSE497J", "Project-I", teams[0], &writer, writer_id),
        (
            "MACSE698",
            "Internship-I/Dissertation-I",
            teams[1],
            &leader_b,
            leader_b_id,
        ),
        ("BCSE4973", "Project-I", teams[2], &leader_c, leader_c_id),
    ] {
        let detail_path = format!("/api/v2/papers/{paper_id}/document-details");
        let paper = state.v2.writer_paper(writer_id, paper_id).await.unwrap();
        let current = test_json(get(&app, &detail_path, Some(&writer.cookie)).await).await;
        assert!(
            current["values"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["field_key"] == "course_name" && item["value"] == expected),
            "{current}"
        );
        let nonleader = test_json(get(&app, &detail_path, Some(&other.cookie)).await).await;
        assert_eq!(nonleader["can_edit"], false);
        for (key, expected_value) in [
            ("course_code", code),
            ("team_academic_year", "2026-2027"),
            ("team_semester", "Fall Semester"),
        ] {
            for details in [&current, &nonleader] {
                assert!(
                    details["values"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|value| value["field_key"] == key && value["value"] == expected_value),
                    "{details}"
                );
                assert_eq!(
                    details["manifest"]["fields"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|field| field["key"] == key)
                        .unwrap()["allow_team_override"],
                    false
                );
            }
        }
        let response = request(&app, Method::PUT, &detail_path, Some(&writer.cookie),
            r#"{"values":{"project_title":"Institutional course acceptance","submission_date":"2026-07-11"},"sections":{}}"#, Some("application/json")).await;
        let status = response.status();
        let saved = test_json(response).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let source = state
            .workspaces
            .read_file(paper.workspace_id, &main_path)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&source);
        assert!(text.contains(&format!("\\newcommand{{\\coursecode}}{{{code}}}")));
        assert!(text.contains(&format!("\\newcommand{{\\coursename}}{{{expected}}}")));
        assert_eq!(text.matches(r"\newcommand{\coursecode}").count(), 1);
        assert_eq!(text.matches(r"\newcommand{\coursename}").count(), 1);
        assert!(text.contains(r"\newcommand{\semester}{Fall Semester 2026-2027}"));
        assert!(text.contains(r"\newcommand{\thesismonth}{July}"));
        assert!(text.contains(r"\newcommand{\thesisyear}{2026}"));
        let reopened = test_json(get(&app, &detail_path, Some(&writer.cookie)).await).await;
        assert!(
            reopened["values"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["field_key"] == "course_name" && item["value"] == expected)
        );
        for _ in 0..3 {
            let response = request(
                &app,
                Method::PUT,
                &detail_path,
                Some(&writer.cookie),
                r#"{"values":{},"sections":{}}"#,
                Some("application/json"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                state
                    .workspaces
                    .read_file(paper.workspace_id, &main_path)
                    .await
                    .unwrap(),
                source
            );
        }
        let exported = export_source_archive(&state, writer_id, paper_id).await;
        assert_eq!(exported.status(), StatusCode::OK);
        let mut zip = zip::ZipArchive::new(Cursor::new(test_bytes(exported).await)).unwrap();
        let mut exported_main = Vec::new();
        zip.by_name(main_name)
            .unwrap()
            .read_to_end(&mut exported_main)
            .unwrap();
        assert_eq!(exported_main, source);
        let response = request(
            &app,
            Method::POST,
            &format!("/api/v2/papers/{paper_id}/builds"),
            Some(&writer.cookie),
            r#"{"trigger_type":"manual"}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let build = test_json(response).await;
        let worker = core_types::WorkerId::new();
        let job = state.queue.claim(worker).await.unwrap().unwrap();
        assert_eq!(job.id.to_string(), build["build_id"].as_str().unwrap());
        let manifest_hash: String = sqlx::query_scalar(
            "SELECT manifest_blob_hash FROM latex_core.snapshots WHERE snapshot_id=$1",
        )
        .bind(job.snapshot_id.to_hex())
        .fetch_one(&pool)
        .await
        .unwrap();
        let manifest: WorkspaceManifestV1 = serde_json::from_slice(
            &state
                .blobs
                .get(core_types::BlobHash::from_str(&manifest_hash).unwrap())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.main_file(), &main_path);
        assert_eq!(
            state
                .blobs
                .get(manifest.files()[&main_path].blob_hash)
                .await
                .unwrap(),
            source
        );
        let execution = compiler
            .compile(
                job.snapshot_id,
                &manifest,
                TexEngine::PdfLatex,
                ShellPolicy::Safe,
                true,
            )
            .await
            .unwrap();
        assert_eq!(
            execution.status(),
            compiler::CompileStatus::Succeeded,
            "{}\n{}",
            String::from_utf8_lossy(execution.stdout()),
            String::from_utf8_lossy(execution.stderr())
        );
        let pdf = execution
            .artifacts()
            .iter()
            .find(|artifact| artifact.kind() == core_types::ArtifactKind::Pdf)
            .unwrap();
        let pdf_dir = tempfile::tempdir().unwrap();
        std::fs::write(pdf_dir.path().join("report.pdf"), pdf.bytes()).unwrap();
        // The existing image runs as UID 10001; expose only this disposable PDF.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(pdf_dir.path(), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            std::fs::set_permissions(
                pdf_dir.path().join("report.pdf"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        let output = Command::new("docker")
            .args([
                "run",
                "--rm",
                "--network=none",
                "--read-only",
                "--cap-drop=ALL",
                "--security-opt=no-new-privileges",
                "--tmpfs",
                "/tmp:rw,noexec,nosuid,size=64m",
                "--mount",
            ])
            .arg(format!(
                "type=bind,src={},dst=/input,readonly",
                pdf_dir.path().display()
            ))
            .args([
                "--entrypoint",
                "gs",
                &image,
                "-q",
                "-dSAFER",
                "-dBATCH",
                "-dNOPAUSE",
                "-sDEVICE=txtwrite",
                "-sOutputFile=-",
                "/input/report.pdf",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let pdf_text = String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(pdf_text.contains(code), "{pdf_text}");
        assert!(pdf_text.contains(expected), "{pdf_text}");
        assert!(!pdf_text.contains("Outdated submitted name"));
        let stored = state.blobs.put(pdf.bytes().clone()).await.unwrap();
        let artifact_id = ArtifactId::new();
        let artifact_manifest = core_types::ArtifactManifestV1::new(
            job.compile_key,
            vec![core_types::ArtifactRefV1 {
                artifact_id,
                kind: core_types::ArtifactKind::Pdf,
                logical_name: pdf.logical_name().clone(),
                blob_hash: stored.hash(),
                size_bytes: stored.size_bytes(),
            }],
        )
        .unwrap();
        let artifact_manifest = state
            .blobs
            .put(Bytes::from(
                artifact_manifest.canonical_json_bytes().unwrap(),
            ))
            .await
            .unwrap();
        state
            .queue
            .complete_success(
                job.id,
                worker,
                &[persistence::PersistedArtifactV1 {
                    artifact_id,
                    kind: core_types::ArtifactKind::Pdf,
                    logical_name: pdf.logical_name().clone(),
                    blob_hash: stored.hash(),
                    size_bytes: stored.size_bytes(),
                    content_type: "application/pdf".into(),
                }],
                artifact_manifest.hash(),
            )
            .await
            .unwrap();
        // Reconcile successful build publication before downloading its artifact.
        let _ = get(
            &app,
            &format!("/api/v2/papers/{paper_id}/builds"),
            Some(&writer.cookie),
        )
        .await;
        let downloaded = get(
            &app,
            &format!(
                "/api/v2/papers/{paper_id}/artifacts/pdf?build={}&download=true",
                job.id
            ),
            Some(&writer.cookie),
        )
        .await;
        assert_eq!(downloaded.status(), StatusCode::OK);
        assert_eq!(
            test_bytes(downloaded).await.as_slice(),
            pdf.bytes().as_ref()
        );
        {
            let browser_worker = queue::CompilationWorker::new(
                state.queue.clone(),
                state.blobs.clone(),
                compiler.clone(),
                core_types::WorkerId::new(),
                queue::WorkerConfig::new(1, Duration::from_millis(100)).unwrap(),
            );
            let shutdown = queue::WorkerShutdown::new();
            let worker_shutdown = shutdown.clone();
            let worker_task = tokio::spawn(async move {
                browser_worker
                    .run_until_shutdown(worker_shutdown)
                    .await
                    .unwrap();
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let served = app.clone();
            let server = tokio::spawn(async move {
                axum::serve(listener, served).await.unwrap();
            });
            let config = serde_json::json!({"base":format!("http://{address}"),"cookie":writer.cookie,"paperId":paper_id,"buildId":job.id,"nonleaderCookie":other.cookie,"registration":{"course":code,"year":"2026-2027","semester":"Fall Semester"},"editableSave":true});
            let output = tokio::task::spawn_blocking(move || {
                Command::new("node")
                    .arg("tests/writer-download-browser.mjs")
                    .env("LATEX_CORE_WRITER_DOWNLOAD_CONFIG", config.to_string())
                    .stdout(std::process::Stdio::inherit())
                    .output()
                    .unwrap()
            })
            .await
            .unwrap();
            shutdown.request();
            worker_task.await.unwrap();
            server.abort();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
        println!(
            "COURSE_PDF: {code}; build={}; snapshot={}; pdf={}",
            job.id,
            job.snapshot_id,
            stored.hash()
        );
        let response = request(
            &app,
            Method::PUT,
            &detail_path,
            Some(&writer.cookie),
            r#"{"values":{"course_name":"Intentional course name edit"},"sections":{}}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        for _ in 0..3 {
            let reopened = test_json(get(&app, &detail_path, Some(&writer.cookie)).await).await;
            assert!(
                reopened["values"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["field_key"] == "course_name"
                        && item["value"] == "Intentional course name edit")
            );
            assert_eq!(
                request(
                    &app,
                    Method::PUT,
                    &detail_path,
                    Some(&writer.cookie),
                    r#"{"values":{},"sections":{}}"#,
                    Some("application/json")
                )
                .await
                .status(),
                StatusCode::OK
            );
        }
        let source = state
            .workspaces
            .read_file(paper.workspace_id, &main_path)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&source);
        assert!(text.contains(r"\newcommand{\coursename}{Intentional course name edit}"));
        assert!(text.contains(r"\newcommand{\thesismonth}{July}"));
        assert_eq!(text.matches(r"\newcommand{\coursename}").count(), 1);
    }
    println!(
        "EXISTING_TEAMS: PASS — A and B existed before Details; distinct Leaders and unchanged registrations; stale overrides ignored; Save and explicit Compile verified in browser."
    );
    let writer = &leader_b;
    let writer_id = leader_b_id;
    let detail_path = format!("/api/v2/papers/{}/document-details", teams[1]);
    // Missing/ambiguous registration blocks a save; no template or old override fallback.
    let current_reg = &postgraduate;
    sqlx::query("INSERT INTO vcap.student_course_registrations(student_reg_no,course_id,academic_year,semester,registration_status) VALUES($1,'SYNTHETIC-AMBIGUOUS','2026-2027','FALL','REGISTERED')").bind(current_reg).execute(&pool).await.unwrap();
    let ambiguous = test_json(get(&app, &detail_path, Some(&writer.cookie)).await).await;
    assert!(
        ambiguous["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("multiple applicable"))
    );
    assert_eq!(
        request(
            &app,
            Method::PUT,
            &detail_path,
            Some(&writer.cookie),
            r#"{"values":{},"sections":{}}"#,
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE vcap.student_user_links SET user_id=NULL,status='UNLINKED',linked_at=NULL WHERE user_id=$1").bind(writer_id.as_uuid()).execute(&pool).await.unwrap();
    let missing = test_json(get(&app, &detail_path, Some(&writer.cookie)).await).await;
    assert!(
        missing["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("no applicable"))
    );
    assert_eq!(
        request(
            &app,
            Method::PUT,
            &detail_path,
            Some(&writer.cookie),
            r#"{"values":{},"sections":{}}"#,
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    pool.close().await;
    database.close().await;
}
