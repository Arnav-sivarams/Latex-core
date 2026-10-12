// Included in database_tests; use only a separate disposable PostgreSQL database.
#[tokio::test]
async fn lossless_template_import_team_details_existing_source_and_snapshot() {
    let _guard = SERVER_TEST_LOCK.lock().await;
    let (database, pool, app, _storage, state) = test_application().await;
    let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
    let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
    let writer_id = test_user_id(&pool, &writer.email).await;
    let mentor_id = test_user_id(&pool, &mentor.email).await;
    let registration = format!("PRESERVE-{}", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO vcap.students(reg_no,name) VALUES($1,'Fixture Student')")
        .bind(&registration)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO vcap.student_user_links(reg_no,user_id,status,linked_at) VALUES($1,$2,'LINKED',now())").bind(&registration).bind(writer_id.as_uuid()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.student_course_registrations(student_reg_no,course_id,academic_year,semester) VALUES($1,'BCSE4973','2026-2027','FALL')").bind(&registration).execute(&pool).await.unwrap();
    let source = front_matter::preservation_tests::sensitive_source("\r\n").replacen(
        "\\documentclass{article}\r\n",
        "\\documentclass{article}\r\n\\newcommand{\\coursecode}{BCSE4973}\r\n\\newcommand{\\semester}{Fall Semester 2026-2027}\r\n\\newcommand{\\academicyear}{2026-2027}\r\n",
        1,
    );
    let page = b"% untouched \\hspace{1cm}\r\n\\hspace{1cm}\\hspace*{2cm}\\vspace{5mm}\r\n\\studentAname  (\\studentAregno) % original spacing\r\n";
    let inputs: &[(&str, &[u8])] = &[
        ("main.tex", source.as_bytes()),
        ("certificate.tex", page),
        ("acknowledgement.tex", page),
        ("chapters/body.tex", b"User-authored \\hspace{1cm}.\r\n"),
        ("assets/data.bin", b"\x00\xff\r\n"),
    ];
    let zip = test_zip(inputs);
    let parsed = archive::read_archive(&zip).unwrap();
    for (name, original) in inputs {
        assert_eq!(
            parsed
                .files
                .iter()
                .find(|file| file.path.as_str() == *name)
                .unwrap()
                .bytes
                .as_ref(),
            *original
        );
    }
    let template_name = format!("Lossless fixture {}", uuid::Uuid::new_v4());
    let (body, content_type) = template_multipart(
        &[
            ("name", template_name.as_str()),
            ("main", "main.tex"),
            ("arrangement", "SINGLE_SOURCE"),
        ],
        &zip,
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
    assert_eq!(response.status(), StatusCode::CREATED);
    let imported = test_json(response).await;
    let template_id = uuid::Uuid::parse_str(imported["id"].as_str().unwrap()).unwrap();
    let records = state.repo.template_files(template_id).await.unwrap();
    for (name, original) in inputs {
        let record = records.iter().find(|file| file.path == *name).unwrap();
        assert_eq!(
            state.blobs.get(record.blob_hash).await.unwrap().as_ref(),
            *original
        );
    }
    let response = request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie), &serde_json::json!({
        "name":"Lossless fixture Team", "writer_ids":[writer_id], "leader_writer_id":writer_id, "mentor_ids":[mentor_id], "template_id":template_id,
    }).to_string(), Some("application/json")).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let team = test_json(response).await;
    let paper_id = uuid::Uuid::parse_str(team["team"]["id"].as_str().unwrap()).unwrap();
    let workspace_id = WorkspaceId::from_uuid(
        uuid::Uuid::parse_str(team["team"]["workspace_id"].as_str().unwrap()).unwrap(),
    );
    assert_eq!(
        state
            .v2
            .paper_template_pin(test_user_id(&pool, &admin.email).await, paper_id)
            .await
            .unwrap()
            .unwrap()["template_id"],
        template_id.to_string()
    );
    for (name, original) in inputs {
        assert_eq!(
            state
                .workspaces
                .read_file(workspace_id, &LogicalPath::parse(name).unwrap())
                .await
                .unwrap()
                .as_ref(),
            *original
        );
    }
    let details = format!("/api/v2/papers/{paper_id}/document-details");
    let opened = get(&app, &details, Some(&writer.cookie)).await;
    assert_eq!(opened.status(), StatusCode::OK);
    assert_eq!(
        state
            .workspaces
            .read_file(workspace_id, &LogicalPath::parse("main.tex").unwrap())
            .await
            .unwrap()
            .as_ref(),
        source.as_bytes()
    );
    let expected = source
        .replacen(r"Old {nested} \{course\} \% name", r"New \& course", 1)
        .replacen("Original title", "New title", 1)
        .replacen("{October}", "{February}", 1)
        .replacen("{2026}", "{2027}", 1)
        .replacen("{2026-10-07}", "{2027-02-09}", 1);
    let body_offset = expected.find(r"\begin{document}").unwrap();
    let mut saved_source = Bytes::new();
    for save in 0..3 {
        let response = request(&app, Method::PUT, &details, Some(&writer.cookie), r#"{"values":{"course_name":"New & course","project_title":"New title","submission_date":"2027-02-09"},"sections":{}}"#, Some("application/json")).await;
        let status = response.status();
        let response = test_json(response).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        let current = state
            .workspaces
            .read_file(workspace_id, &LogicalPath::parse("main.tex").unwrap())
            .await
            .unwrap();
        let text = std::str::from_utf8(&current).unwrap();
        let header = text
            .find("% Known institutional single-source bindings.\r\n")
            .unwrap();
        let document = text.find(r"\begin{document}").unwrap();
        assert_eq!(&text[..header], &expected[..body_offset]);
        assert_eq!(&text[document..], &expected[body_offset..]);
        if save > 0 {
            assert_eq!(current, saved_source);
        }
        saved_source = current;
        for (name, original) in inputs.iter().skip(1) {
            assert_eq!(
                state
                    .workspaces
                    .read_file(workspace_id, &LogicalPath::parse(name).unwrap())
                    .await
                    .unwrap()
                    .as_ref(),
                *original
            );
        }
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM latex_core.compile_jobs WHERE workspace_id=$1"
        )
        .bind(workspace_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    // Simulate an existing Team's legitimate editor save, then reopen Details.
    let file = state
        .v2
        .visible_paper_files(workspace_id)
        .await
        .unwrap()
        .into_iter()
        .find(|file| file.path.as_str() == "main.tex")
        .unwrap();
    let manual = std::str::from_utf8(&saved_source)
        .unwrap()
        .replacen("{2027-02-09}", "{2028-03-10}", 1)
        .replacen(
            r"\end{document}",
            "  % manual \\hspace{1cm} \\linespread{1.7}\r\n\\end{document}",
            1,
        );
    let response = request(&app, Method::PUT, &format!("/api/v2/papers/{paper_id}/files/{}", file.file_id), Some(&writer.cookie), &serde_json::json!({"content":manual,"version":state.workspaces.restore(workspace_id).await.unwrap().version().get()}).to_string(), Some("application/json")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let reopened = test_json(get(&app, &details, Some(&writer.cookie)).await).await;
    assert!(
        reopened["values"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["field_key"] == "submission_date" && field["value"] == "2028-03-10"),
        "manual date helper must take precedence over stale saved metadata"
    );
    let response = request(
        &app,
        Method::PUT,
        &details,
        Some(&writer.cookie),
        r#"{"values":{"project_title":"New title"},"sections":{}}"#,
        Some("application/json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let canonical = state
        .workspaces
        .read_file(workspace_id, &LogicalPath::parse("main.tex").unwrap())
        .await
        .unwrap();
    assert_eq!(canonical.as_ref(), manual.as_bytes());
    let checkpoint = state.workspaces.force_snapshot(workspace_id).await.unwrap();
    let manifest: WorkspaceManifestV1 = serde_json::from_slice(
        &state
            .blobs
            .get(checkpoint.manifest_blob_hash())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        state
            .blobs
            .get(manifest.files()[&LogicalPath::parse("main.tex").unwrap()].blob_hash)
            .await
            .unwrap(),
        canonical
    );
    for (name, original) in inputs {
        let record = records.iter().find(|file| file.path == *name).unwrap();
        assert_eq!(
            state.blobs.get(record.blob_hash).await.unwrap().as_ref(),
            *original
        );
    }
    if env::var_os("LATEX_CORE_PRESERVATION_TEXLIVE_IMAGE").is_some() {
        let materialized = vec![archive::ImportedFile {
            path: LogicalPath::parse("main.tex").unwrap(),
            bytes: canonical,
        }];
        // Independent baseline uses the intended values and identical manual edit,
        // with no generated metadata block or application binding.
        let baseline = expected
            .replacen("{2027-02-09}", "{2028-03-10}", 1)
            .replacen(
                r"\end{document}",
                "  % manual \\hspace{1cm} \\linespread{1.7}\r\n\\end{document}",
                1,
            );
        let original = vec![archive::ImportedFile {
            path: LogicalPath::parse("main.tex").unwrap(),
            bytes: Bytes::from(baseline),
        }];
        preservation_compile_tests::compare_layout(
            "api-sensitive-source",
            &original,
            &materialized,
            &LogicalPath::parse("main.tex").unwrap(),
        )
        .await;
    }
    pool.close().await;
    database.close().await;
}
