// Called by the disposable real-browser fixture, with the actual institutional ZIP.
use sha2::Digest as _;
async fn fidelity_sources(state: &AppState, workspace: WorkspaceId) -> Vec<archive::ImportedFile> {
    let mut files = Vec::new();
    for file in state.v2.visible_paper_files(workspace).await.unwrap() {
        files.push(archive::ImportedFile {
            bytes: state
                .workspaces
                .read_file(workspace, &file.path)
                .await
                .unwrap(),
            path: file.path,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn record_fidelity_stage(label: &str, files: &[archive::ImportedFile]) {
    let root =
        std::path::PathBuf::from(env::var_os("LATEX_CORE_FIDELITY_OUTPUT").unwrap()).join(label);
    let mut hashes = BTreeMap::new();
    for file in files {
        let path = root.join(file.path.as_str());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &file.bytes).unwrap();
        hashes.insert(
            file.path.as_str(),
            hex::encode(sha2::Sha256::digest(&file.bytes)),
        );
    }
    std::fs::write(
        root.join("sha256.json"),
        serde_json::to_vec_pretty(&hashes).unwrap(),
    )
    .unwrap();
}

async fn verify_actual_template_fidelity(
    app: &Router,
    state: &AppState,
    pool: &PgPool,
    writer: &Fixture,
    team: &serde_json::Value,
    main: &str,
) {
    let paper = uuid::Uuid::parse_str(team.as_str().unwrap()).unwrap();
    let workspace = WorkspaceId::from_uuid(
        sqlx::query_scalar("SELECT workspace_id FROM latex_core.paper_teams WHERE id=$1")
            .bind(paper)
            .fetch_one(pool)
            .await
            .unwrap(),
    );
    let template: uuid::Uuid = sqlx::query_scalar(
        "SELECT template_id FROM latex_core.paper_template_pins WHERE paper_id=$1",
    )
    .bind(paper)
    .fetch_one(pool)
    .await
    .unwrap();
    let original = archive::read_archive(include_bytes!(
        "../../../artifacts/Full_Report_template_v1.1.zip"
    ))
    .unwrap()
    .files;
    record_fidelity_stage("01-original-zip", &original);
    let mut pinned = Vec::new();
    for file in state.repo.template_files(template).await.unwrap() {
        pinned.push(archive::ImportedFile {
            path: LogicalPath::parse(&file.path).unwrap(),
            bytes: state.blobs.get(file.blob_hash).await.unwrap(),
        });
    }
    record_fidelity_stage("02-stored-pin", &pinned);
    let created = fidelity_sources(state, workspace).await;
    record_fidelity_stage("03-materialized-team", &created);
    for source in &original {
        for stage in [&pinned, &created] {
            assert_eq!(
                stage.iter().find(|f| f.path == source.path).unwrap().bytes,
                source.bytes
            );
        }
    }
    let details = format!("/api/v2/papers/{paper}/document-details");
    record_fidelity_stage("04-before-details", &created);
    assert_eq!(
        get(app, &details, Some(&writer.cookie)).await.status(),
        StatusCode::OK
    );
    let opened = fidelity_sources(state, workspace).await;
    record_fidelity_stage("05-after-details-get", &opened);
    for (a, b) in created.iter().zip(&opened) {
        assert_eq!(a.bytes, b.bytes);
    }
    let mut last: Vec<archive::ImportedFile> = Vec::new();
    for iteration in 0..3 {
        let response = request(app, Method::PUT, &details, Some(&writer.cookie),
            r#"{"values":{"project_title":"Template fidelity fixture","submission_date":"2026-10-20"},"sections":{}}"#, Some("application/json")).await;
        let status = response.status();
        assert_eq!(status, StatusCode::OK, "{}", test_json(response).await);
        let saved = fidelity_sources(state, workspace).await;
        if iteration > 0 {
            for (a, b) in saved.iter().zip(&last) {
                assert_eq!(a.bytes, b.bytes);
            }
        }
        last = saved;
    }
    record_fidelity_stage("06-repeated-details-save", &last);
    // Independently allow only the original one-line metadata definition values
    // and new allowlisted metadata definitions. Every other byte must match.
    let original_main = &original
        .iter()
        .find(|f| f.path.as_str() == main)
        .unwrap()
        .bytes;
    let saved_main = &last.iter().find(|f| f.path.as_str() == main).unwrap().bytes;
    let saved_text = std::str::from_utf8(saved_main).unwrap();
    let mut baseline = String::new();
    for line in std::str::from_utf8(original_main)
        .unwrap()
        .split_inclusive('\n')
    {
        let mut expected = line.to_owned();
        for (command, _, _, _) in front_matter::legacy::REGISTRY {
            let prefix = format!("\\newcommand{{\\{command}}}{{");
            if let Some(value) = line.strip_prefix(&prefix) {
                let end = value.find('}').unwrap();
                let new_line = saved_text
                    .split_inclusive('\n')
                    .find(|line| line.starts_with(&prefix))
                    .unwrap();
                let new_value = &new_line[prefix.len()..];
                let new_end = new_value.find('}').unwrap();
                assert_eq!(
                    &value[end..],
                    &new_value[new_end..],
                    "metadata suffix/comments changed"
                );
                expected = new_line.to_owned();
            }
        }
        baseline.push_str(&expected);
    }
    let header = "% Known institutional single-source bindings.\n";
    let start = saved_text.find(header).unwrap();
    let end = saved_text.find("\\begin{document}").unwrap();
    let additions = &saved_text[start..end];
    for line in additions.lines().skip(1) {
        if line == front_matter::SINGLE_SOURCE_MARKER {
            continue;
        }
        let name = line
            .strip_prefix("\\newcommand{\\")
            .unwrap()
            .split_once("}{")
            .unwrap()
            .0;
        assert!(
            front_matter::legacy::REGISTRY
                .iter()
                .any(|(command, _, _, _)| *command == name)
                || [
                    "latexcoresubmissiondate",
                    "latexcoresemester",
                    "latexcoreacademicyear"
                ]
                .contains(&name)
        );
    }
    baseline = baseline.replacen(
        "\\begin{document}",
        &format!("{additions}\\begin{{document}}"),
        1,
    );
    assert_eq!(
        baseline.as_bytes(),
        saved_main.as_ref(),
        "unauthorized main-source change"
    );
    for source in original.iter().filter(|f| f.path.as_str() != main) {
        assert_eq!(
            last.iter().find(|f| f.path == source.path).unwrap().bytes,
            source.bytes
        );
    }
    let certificate = "Full_Report_template_v1.0/certificate.tex";
    let file = state
        .v2
        .visible_paper_files(workspace)
        .await
        .unwrap()
        .into_iter()
        .find(|f| f.path.as_str() == certificate)
        .unwrap();
    let authored =
        std::str::from_utf8(&original.iter().find(|f| f.path == file.path).unwrap().bytes).unwrap();
    // A real manual spacing edit must survive another Details save.
    let manual = authored.replacen(r"\hspace{1cm}", r"\hspace{1.25cm}", 1);
    let version = state
        .workspaces
        .restore(workspace)
        .await
        .unwrap()
        .version()
        .get();
    assert_eq!(
        request(
            app,
            Method::PUT,
            &format!("/api/v2/papers/{paper}/files/{}", file.file_id),
            Some(&writer.cookie),
            &serde_json::json!({"content":manual,"version":version}).to_string(),
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(
            app,
            Method::PUT,
            &details,
            Some(&writer.cookie),
            r#"{"values":{"project_title":"Template fidelity fixture"},"sections":{}}"#,
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        state
            .workspaces
            .read_file(workspace, &file.path)
            .await
            .unwrap()
            .as_ref(),
        manual.as_bytes()
    );
    let final_files = fidelity_sources(state, workspace).await;
    record_fidelity_stage("07-manual-hspace-and-save", &final_files);
    let snapshot = state.workspaces.force_snapshot(workspace).await.unwrap();
    let manifest: WorkspaceManifestV1 = serde_json::from_slice(
        &state
            .blobs
            .get(snapshot.manifest_blob_hash())
            .await
            .unwrap(),
    )
    .unwrap();
    let mut compiler_input = Vec::new();
    for (path, entry) in manifest.files() {
        compiler_input.push(archive::ImportedFile {
            path: path.clone(),
            bytes: state.blobs.get(entry.blob_hash).await.unwrap(),
        });
    }
    record_fidelity_stage("08-compiler-input", &compiler_input);
    for (a, b) in final_files.iter().zip(&compiler_input) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.bytes, b.bytes);
    }
    let mut baseline_files = original.clone();
    baseline_files
        .iter_mut()
        .find(|f| f.path.as_str() == main)
        .unwrap()
        .bytes = Bytes::from(baseline);
    baseline_files
        .iter_mut()
        .find(|f| f.path.as_str() == certificate)
        .unwrap()
        .bytes = Bytes::from(manual.clone());
    preservation_compile_tests::compare_layout(
        "actual-zip-api-metadata-manual-edit",
        &baseline_files,
        &compiler_input,
        &LogicalPath::parse(main).unwrap(),
    )
    .await;
    // Existing historical source is preserved until its owner explicitly edits it.
    let historical = format!(
        "{}\n% Later legitimate author edit\n",
        manual.replace(r"\hspace{1cm}", r"\setlength{\parindent}{1cm}\indent")
    );
    let version = state
        .workspaces
        .restore(workspace)
        .await
        .unwrap()
        .version()
        .get();
    assert_eq!(
        request(
            app,
            Method::PUT,
            &format!("/api/v2/papers/{paper}/files/{}", file.file_id),
            Some(&writer.cookie),
            &serde_json::json!({"content":historical,"version":version}).to_string(),
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    record_fidelity_stage(
        "09-historical-fixture-before",
        &fidelity_sources(state, workspace).await,
    );
    assert_eq!(
        get(app, &details, Some(&writer.cookie)).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        request(
            app,
            Method::PUT,
            &details,
            Some(&writer.cookie),
            r#"{"values":{"project_title":"Template fidelity fixture"},"sections":{}}"#,
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        state
            .workspaces
            .read_file(workspace, &file.path)
            .await
            .unwrap()
            .as_ref(),
        historical.as_bytes()
    );
    record_fidelity_stage(
        "10-historical-fixture-after",
        &fidelity_sources(state, workspace).await,
    );
    // Simulate owner approval only in this disposable fixture: restore the
    // known authored spacing, retaining the later manual edit and comment.
    let restored = format!("{manual}\n% Later legitimate author edit\n");
    let version = state
        .workspaces
        .restore(workspace)
        .await
        .unwrap()
        .version()
        .get();
    assert_eq!(
        request(
            app,
            Method::PUT,
            &format!("/api/v2/papers/{paper}/files/{}", file.file_id),
            Some(&writer.cookie),
            &serde_json::json!({"content":restored,"version":version}).to_string(),
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(
            app,
            Method::PUT,
            &details,
            Some(&writer.cookie),
            r#"{"values":{"project_title":"Template fidelity fixture"},"sections":{}}"#,
            Some("application/json")
        )
        .await
        .status(),
        StatusCode::OK
    );
    let recovered = fidelity_sources(state, workspace).await;
    assert_eq!(
        recovered
            .iter()
            .find(|f| f.path == file.path)
            .unwrap()
            .bytes
            .as_ref(),
        restored.as_bytes()
    );
    record_fidelity_stage("11-restored-disposable-fixture", &recovered);
    let history_count: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.workspace_events WHERE workspace_id=$1 AND payload::text LIKE $2").bind(workspace.as_uuid()).bind(format!("%{}%",hex::encode(sha2::Sha256::digest(historical.as_bytes())))).fetch_one(pool).await.unwrap();
    assert!(
        history_count > 0,
        "restoration must retain the earlier revision"
    );
    preservation_compile_tests::compare_layout(
        "historical-fixture-restored",
        &baseline_files,
        &recovered,
        &LogicalPath::parse(main).unwrap(),
    )
    .await;
    println!(
        "TEMPLATE_FIDELITY_PASS: actual ZIP, pin, Team, read-only GET, repeated saves, manual hspace edit, historical fixture, compiler snapshot and identical PDF layout"
    );
}
