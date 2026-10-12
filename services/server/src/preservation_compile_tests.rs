//! Opt-in tests use the frozen sandbox and compare every rendered PDF page.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "isolated test assertions"
)]

use super::*;
use crate::archive::ImportedFile;
use core_types::{ArtifactKind, FileEntryV1};

const FROZEN_IMAGE: &str =
    "sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38";

async fn compile_pdf(files: &[ImportedFile], main: &LogicalPath) -> Bytes {
    let image = env::var("LATEX_CORE_PRESERVATION_TEXLIVE_IMAGE").expect("frozen image required");
    assert_eq!(image, FROZEN_IMAGE, "tests must use the frozen M7 image");
    let storage = tempfile::tempdir().unwrap();
    let store = Arc::new(
        FsBlobStore::open(
            storage.path().join("blobs"),
            FsBlobStoreConfig::development_default(),
        )
        .await
        .unwrap(),
    );
    let mut entries = BTreeMap::new();
    for file in files {
        let stored = store.put(file.bytes.clone()).await.unwrap();
        entries.insert(
            file.path.clone(),
            FileEntryV1 {
                blob_hash: stored.hash(),
                size_bytes: stored.size_bytes(),
            },
        );
    }
    let manifest = WorkspaceManifestV1::new(main.clone(), entries).unwrap();
    let compiler = compiler::CompilerService::new(
        store,
        compiler::DockerCliRuntime::new(image).unwrap(),
        compiler::CompilerConfig::new(compiler::CompileLimits::development_default())
            .with_staging_root(storage.path().join("staging")),
    )
    .unwrap();
    let result = compiler
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
        result.status(),
        compiler::CompileStatus::Succeeded,
        "{}\n{}",
        String::from_utf8_lossy(result.stdout()),
        String::from_utf8_lossy(result.stderr())
    );
    result
        .artifacts()
        .iter()
        .find(|artifact| artifact.kind() == ArtifactKind::Pdf)
        .expect("PDF")
        .bytes()
        .clone()
}

async fn render_pdf(pdf: Bytes, path: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(path.join("report.pdf"), pdf).unwrap();
    // Run the PDF interpreter with no network, privileges or deployment mounts.
    // Use the host test user to write only this private output directory.
    // The container has no capabilities and no other writable host mount.
    let uid = Command::new("id").arg("-u").output().unwrap();
    assert!(uid.status.success());
    let user = format!("--user={}", String::from_utf8(uid.stdout).unwrap().trim());
    let result = tokio::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--network=none",
            "--read-only",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--pids-limit=32",
            "--memory=512m",
            "--cpus=1",
            "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size=64m",
            "--mount",
        ])
        .arg(format!(
            "type=bind,source={},target=/comparison",
            path.display()
        ))
        .arg(user)
        .args([
            "--entrypoint=gs",
            FROZEN_IMAGE,
            "-q",
            "-dSAFER",
            "-dBATCH",
            "-dNOPAUSE",
            "-sDEVICE=png16m",
            "-r96",
            "-dTextAlphaBits=4",
            "-dGraphicsAlphaBits=4",
            "-sOutputFile=/comparison/page-%04d.png",
            "/comparison/report.pdf",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut pages = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|page| page.extension().is_some_and(|extension| extension == "png"))
        .collect::<Vec<_>>();
    pages.sort();
    assert!(
        !pages.is_empty(),
        "renderer emitted no pages: {} / {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    pages
}

pub(crate) async fn compare_layout(
    label: &str,
    original: &[ImportedFile],
    materialized: &[ImportedFile],
    main: &LogicalPath,
) {
    let temporary = tempfile::tempdir().unwrap();
    let root = env::var_os("LATEX_CORE_PRESERVATION_OUTPUT").map_or_else(
        || temporary.path().join(label),
        |path| std::path::PathBuf::from(path).join(label),
    );
    let baseline = render_pdf(compile_pdf(original, main).await, &root.join("original")).await;
    let imported = render_pdf(
        compile_pdf(materialized, main).await,
        &root.join("materialized"),
    )
    .await;
    assert_eq!(baseline.len(), imported.len(), "page count changed");
    for (index, (before, after)) in baseline.iter().zip(&imported).enumerate() {
        let before = image::open(before).unwrap().into_rgb8();
        let after = image::open(after).unwrap().into_rgb8();
        assert_eq!(
            before.dimensions(),
            after.dimensions(),
            "page {} dimensions changed",
            index + 1
        );
        assert!(
            before.as_raw() == after.as_raw(),
            "page {} text positioning, spacing or margins changed",
            index + 1
        );
    }
    println!(
        "{label}: original and materialized compile; {} pages, dimensions and all pixels identical at 96 dpi",
        baseline.len()
    );
}

#[tokio::test]
#[ignore = "requires explicit frozen Docker image; renders original private template copies"]
async fn original_institutional_zip_compiles_with_identical_imported_layout() {
    let zip = include_bytes!("../../../artifacts/Full_Report_template_v1.1.zip");
    let mut archive = archive::read_archive(zip).unwrap();
    let main = LogicalPath::parse("Full_Report_template_v1.0/Full_Report_v1.0.tex").unwrap();
    let mut imported = archive.files.clone();
    let source = imported.iter_mut().find(|file| file.path == main).unwrap();
    let values = front_matter::single_source_values_from_tex(&source.bytes);
    let identical_metadata = BTreeMap::from([
        ("course_name".into(), values["course_name"].clone()),
        ("project.title".into(), values["project.title"].clone()),
    ]);
    source.bytes =
        front_matter::bind_single_source_values(&source.bytes, &identical_metadata).unwrap();
    for (original, materialized) in archive.files.iter().zip(&imported) {
        assert_eq!(
            original.bytes, materialized.bytes,
            "{} changed",
            original.path
        );
    }
    // The archive references \academicyear without defining it. Supply the
    // same metadata through a disposable wrapper on both sides; every byte of
    // every original archive entry remains unchanged. This is not a claim that
    // the raw ZIP compiles without the missing metadata.
    let harness = ImportedFile {
        path: LogicalPath::parse("Full_Report_template_v1.0/preservation-baseline.tex").unwrap(),
        bytes: Bytes::from_static(
            b"\\providecommand{\\academicyear}{2026--27}\n\\input{Full_Report_v1.0.tex}\n",
        ),
    };
    archive.files.push(harness.clone());
    imported.push(harness.clone());
    compare_layout(
        "institutional-zip-identical-metadata",
        &archive.files,
        &imported,
        &harness.path,
    )
    .await;
}
