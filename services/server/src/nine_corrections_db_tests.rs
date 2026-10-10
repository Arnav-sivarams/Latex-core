// Actual Axum, PostgreSQL, Yjs browser, and frozen compiler acceptance.
#[tokio::test]
async fn nine_corrections_browser_and_compiler_acceptance() {
    let _guard = SERVER_TEST_LOCK.lock().await;
    let (database, pool, _app, _storage, mut state) = test_application().await;
    state.mail_enabled = false;
    state.mail_outbox = None;
    state.repo = AppRepository::new(database.clone());
    state.institution = InstitutionRepository::new(database.clone());
    let image = env::var("LATEX_CORE_COURSE_TEXLIVE_IMAGE").expect("Frozen M7 image required");
    let staging = tempfile::tempdir().unwrap();
    let compiler = Arc::new(
        compiler::CompilerService::new(
            state.blobs.clone(),
            compiler::DockerCliRuntime::new(image).unwrap(),
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
    let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
    let outsider = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
    let writer_id = test_user_id(&pool, &writer.email).await;
    let other_id = test_user_id(&pool, &other.email).await;
    let mentor_id = test_user_id(&pool, &mentor.email).await;
    fixture_default_template(&state, &pool, &admin).await;
    let name = "Smart Energy Monitoring 研究";
    let response = request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
        &serde_json::json!({"name":name,"writer_ids":[writer_id,other_id],"leader_writer_id":writer_id,"mentor_ids":[mentor_id]}).to_string(), Some("application/json")).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = test_json(response).await;
    let paper_id = created["team"]["id"].as_str().unwrap();
    // Deliberately long synthetic identities exercise layout without personal data.
    for (id, label) in [
        (writer_id, "leader"),
        (other_id, "writer"),
        (mentor_id, "mentor"),
    ] {
        sqlx::query("UPDATE latex_core.user_credentials SET email=$1 WHERE user_id=$2")
            .bind(format!(
                "{label}.{}@nine-corrections.example.invalid",
                "long-address-".repeat(8)
            ))
            .bind(id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
    }
    let email: String =
        sqlx::query_scalar("SELECT email FROM latex_core.user_credentials WHERE user_id=$1")
            .bind(writer_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let generated = Command::new("node")
        .arg("tests/nine-corrections-snippets.mjs")
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let snippets: serde_json::Value = serde_json::from_slice(&generated.stdout).unwrap();
    let mut logo = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut logo, ImageFormat::Png)
        .unwrap();
    let logo = Bytes::from(logo.into_inner());
    for mode in ["source", "legacy"] {
        let mut files = BTreeMap::new();
        for (path, bytes) in [
            (
                "main.tex",
                Bytes::from(snippets[mode].as_str().unwrap().to_owned()),
            ),
            (
                "references.bib",
                Bytes::from(snippets["bib"].as_str().unwrap().to_owned()),
            ),
            (
                "data.csv",
                Bytes::from(snippets["csv"].as_str().unwrap().to_owned()),
            ),
            ("logo.png", logo.clone()),
        ] {
            let blob = state.blobs.put(bytes).await.unwrap();
            files.insert(
                LogicalPath::parse(path).unwrap(),
                core_types::FileEntryV1 {
                    blob_hash: blob.hash(),
                    size_bytes: blob.size_bytes(),
                },
            );
        }
        let manifest =
            WorkspaceManifestV1::new(LogicalPath::parse("main.tex").unwrap(), files).unwrap();
        let execution = compiler
            .compile(
                manifest.snapshot_id().unwrap(),
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
            "{mode}: {} {}",
            String::from_utf8_lossy(execution.stdout()),
            String::from_utf8_lossy(execution.stderr())
        );
        println!(
            "NINE_COMPILER_PASS: {mode}; categories={}; symbols={}",
            snippets["categories"], snippets["symbols"]
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, served).await.unwrap();
    });
    let worker = queue::CompilationWorker::new(
        state.queue.clone(),
        state.blobs.clone(),
        compiler.clone(),
        core_types::WorkerId::new(),
        queue::WorkerConfig::new(1, Duration::from_millis(100)).unwrap(),
    );
    let shutdown = queue::WorkerShutdown::new();
    let worker_shutdown = shutdown.clone();
    let worker_task = tokio::spawn(async move {
        worker.run_until_shutdown(worker_shutdown).await.unwrap();
    });
    let config = serde_json::json!({"base":format!("http://{address}"),"paperId":paper_id,"teamName":name,"writerCookie":writer.cookie,"otherCookie":other.cookie,"mentorCookie":mentor.cookie,"outsiderCookie":outsider.cookie,"adminCookie":admin.cookie,"writerEmail":email,"password":PASSWORD});
    let browser_config = config.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new("node")
            .arg("tests/nine-corrections-browser.mjs")
            .env("LATEX_CORE_NINE_CONFIG", browser_config.to_string())
            .stdout(std::process::Stdio::inherit())
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    shutdown.request();
    worker_task.await.unwrap();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Stop the test worker before enqueueing so cancellation is deterministic.
    let submitted = request(
        &app,
        Method::POST,
        &format!("/api/v2/papers/{paper_id}/builds"),
        Some(&writer.cookie),
        r#"{"trigger_type":"manual"}"#,
        Some("application/json"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::ACCEPTED);
    let submitted = test_json(submitted).await;
    let build_id = uuid::Uuid::parse_str(submitted["build_id"].as_str().unwrap()).unwrap();
    let job_id: uuid::Uuid =
        sqlx::query_scalar("SELECT compile_job_id FROM latex_core.v2_paper_builds WHERE id=$1")
            .bind(build_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command as AsyncCommand;
    let mut browser = AsyncCommand::new("node")
        .arg("tests/nine-corrections-cancellation-browser.mjs")
        .env("LATEX_CORE_NINE_CONFIG", config.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(browser.stdout.take().unwrap()).lines();
    let ready = tokio::time::timeout(Duration::from_secs(45), lines.next_line())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready.as_deref(), Some("CANCELLATION_READY"));
    state
        .queue
        .request_cancellation(
            JobId::from_uuid(job_id),
            Some("disposable acceptance cancellation"),
        )
        .await
        .unwrap();
    browser
        .stdin
        .take()
        .unwrap()
        .write_all(b"cancelled\n")
        .await
        .unwrap();
    let completed = tokio::time::timeout(Duration::from_secs(45), browser.wait())
        .await
        .unwrap()
        .unwrap();
    while let Some(line) = lines.next_line().await.unwrap() {
        println!("{line}");
    }
    assert!(
        completed.success(),
        "cancellation browser acceptance failed"
    );
    server.abort();

    pool.close().await;
    database.close().await;
}
