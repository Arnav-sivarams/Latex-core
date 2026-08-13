use blob_store::{BlobStore, FsBlobStore, FsBlobStoreConfig};
use bytes::Bytes;
use compiler::{
    CompileContainerRequest, CompileLimits, CompilerConfig, CompilerError, CompilerService,
    ContainerOutput, ContainerRuntime,
};
use core_types::{BlobHash, FileEntryV1, LogicalPath, ShellPolicy, TexEngine, WorkspaceManifestV1};
use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tex_index::{TexEnvironmentIndexV1, TexLiveRelease};

type SeenFiles = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

#[derive(Clone)]
struct InspectRuntime {
    probe: Vec<u8>,
    seen: SeenFiles,
}
impl ContainerRuntime for InspectRuntime {
    fn probe_image(&self) -> Result<Vec<u8>, CompilerError> {
        Ok(self.probe.clone())
    }
    fn execute(&self, request: &CompileContainerRequest) -> Result<ContainerOutput, CompilerError> {
        let mut seen = self
            .seen
            .lock()
            .map_err(|_| CompilerError::InternalInvariant {
                message: "lock".into(),
            })?;
        collect(request.workspace(), request.workspace(), &mut seen);
        fs::write(request.workspace().join(".latex-core-out/main.pdf"), b"pdf").expect("write pdf");
        Ok(ContainerOutput {
            exit_code: Some(0),
            timed_out: false,
            stdout: Bytes::new(),
            stderr: Bytes::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }
}

#[derive(Clone)]
struct OutputRuntime {
    probe: Vec<u8>,
    exit_code: i32,
    write_output: fn(&std::path::Path),
}

impl ContainerRuntime for OutputRuntime {
    fn probe_image(&self) -> Result<Vec<u8>, CompilerError> {
        Ok(self.probe.clone())
    }

    fn execute(&self, request: &CompileContainerRequest) -> Result<ContainerOutput, CompilerError> {
        (self.write_output)(&request.workspace().join(".latex-core-out"));
        Ok(ContainerOutput {
            exit_code: Some(self.exit_code),
            timed_out: false,
            stdout: Bytes::new(),
            stderr: Bytes::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }
}

#[derive(Clone)]
struct BlockingRuntime {
    probe: Vec<u8>,
    started: Arc<AtomicBool>,
    timer_fired: Arc<AtomicBool>,
}

impl ContainerRuntime for BlockingRuntime {
    fn probe_image(&self) -> Result<Vec<u8>, CompilerError> {
        Ok(self.probe.clone())
    }

    fn execute(&self, request: &CompileContainerRequest) -> Result<ContainerOutput, CompilerError> {
        self.started.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(1);
        while !self.timer_fired.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                return Err(CompilerError::InternalInvariant {
                    message: "current-thread executor did not progress during runtime execution"
                        .into(),
                });
            }
            thread::sleep(Duration::from_millis(5));
        }
        fs::write(request.workspace().join(".latex-core-out/main.pdf"), b"pdf").expect("write pdf");
        Ok(ContainerOutput {
            exit_code: Some(0),
            timed_out: false,
            stdout: Bytes::new(),
            stderr: Bytes::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }
}
fn collect(root: &std::path::Path, path: &std::path::Path, seen: &mut Vec<(String, Vec<u8>)>) {
    for entry in fs::read_dir(path).expect("read").flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == ".latex-core-out") {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, seen);
        } else {
            seen.push((
                path.strip_prefix(root)
                    .expect("relative")
                    .to_string_lossy()
                    .into_owned(),
                fs::read(path).expect("bytes"),
            ));
        }
    }
}
fn probe() -> Vec<u8> {
    TexEnvironmentIndexV1::new(
        TexLiveRelease::new(2026, "x86_64-linux".into()).expect("release"),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .expect("index")
    .canonical_json_bytes()
    .expect("json")
}

#[tokio::test]
async fn materializes_nested_binary_unicode_spaces_zero_and_deduplicated_content() {
    let directory = tempfile::tempdir().expect("temp");
    let store = FsBlobStore::open(
        directory.path().join("blobs"),
        FsBlobStoreConfig::development_default(),
    )
    .await
    .expect("store");
    let shared = Bytes::from_static(b"shared");
    let binary = Bytes::from_static(&[0, 1, 2, 255]);
    let shared_put = store.put(shared.clone()).await.expect("put");
    let binary_put = store.put(binary.clone()).await.expect("put");
    let zero_put = store.put(Bytes::new()).await.expect("put");
    let mut files = BTreeMap::new();
    for path in ["main.tex", "nested/one.tex", "unicodé/文 件.tex"] {
        files.insert(
            LogicalPath::parse(path).expect("path"),
            FileEntryV1 {
                blob_hash: shared_put.hash(),
                size_bytes: shared_put.size_bytes(),
            },
        );
    }
    files.insert(
        LogicalPath::parse("images/pixel.png").expect("path"),
        FileEntryV1 {
            blob_hash: binary_put.hash(),
            size_bytes: binary_put.size_bytes(),
        },
    );
    files.insert(
        LogicalPath::parse("empty file.txt").expect("path"),
        FileEntryV1 {
            blob_hash: zero_put.hash(),
            size_bytes: 0,
        },
    );
    let manifest = WorkspaceManifestV1::new(LogicalPath::parse("main.tex").expect("main"), files)
        .expect("manifest");
    let snapshot = manifest.snapshot_id().expect("snapshot");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let runtime = InspectRuntime {
        probe: probe(),
        seen: Arc::clone(&seen),
    };
    let service = CompilerService::new(
        Arc::new(store),
        runtime,
        CompilerConfig::new(CompileLimits::development_default()),
    )
    .expect("service");
    service
        .compile(
            snapshot,
            &manifest,
            TexEngine::PdfLatex,
            ShellPolicy::Safe,
            false,
        )
        .await
        .expect("compile");
    let seen = seen.lock().expect("seen");
    assert_eq!(seen.len(), 5);
    assert!(
        seen.iter()
            .any(|(path, bytes)| path == "images/pixel.png" && bytes == binary.as_ref())
    );
    assert!(
        seen.iter()
            .any(|(path, bytes)| path == "empty file.txt" && bytes.is_empty())
    );
}

#[tokio::test]
async fn rejects_snapshot_and_size_mismatches() {
    let directory = tempfile::tempdir().expect("temp");
    let store = FsBlobStore::open(directory.path(), FsBlobStoreConfig::development_default())
        .await
        .expect("store");
    let put = store.put(Bytes::from_static(b"x")).await.expect("put");
    let mut files = BTreeMap::new();
    files.insert(
        LogicalPath::parse("main.tex").expect("path"),
        FileEntryV1 {
            blob_hash: put.hash(),
            size_bytes: 2,
        },
    );
    let manifest = WorkspaceManifestV1::new(LogicalPath::parse("main.tex").expect("path"), files)
        .expect("manifest");
    let snapshot = manifest.snapshot_id().expect("snapshot");
    let runtime = InspectRuntime {
        probe: probe(),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let service = CompilerService::new(
        Arc::new(store),
        runtime,
        CompilerConfig::new(CompileLimits::development_default()),
    )
    .expect("service");
    let wrong: core_types::SnapshotId = "00".repeat(32).parse().expect("id");
    assert!(matches!(
        service
            .compile(
                wrong,
                &manifest,
                TexEngine::PdfLatex,
                ShellPolicy::Safe,
                false
            )
            .await,
        Err(CompilerError::SnapshotMismatch { .. })
    ));
    assert!(matches!(
        service
            .compile(
                snapshot,
                &manifest,
                TexEngine::PdfLatex,
                ShellPolicy::Safe,
                false
            )
            .await,
        Err(CompilerError::Materialization { .. })
    ));
    let _hash = BlobHash::digest(b"x");
}

#[tokio::test(flavor = "current_thread")]
async fn blocking_container_runtime_does_not_stall_the_current_thread_executor() {
    let directory = tempfile::tempdir().expect("temp");
    let store = FsBlobStore::open(directory.path(), FsBlobStoreConfig::development_default())
        .await
        .expect("store");
    let put = store.put(Bytes::from_static(b"x")).await.expect("put");
    let mut files = BTreeMap::new();
    files.insert(
        LogicalPath::parse("main.tex").expect("path"),
        FileEntryV1 {
            blob_hash: put.hash(),
            size_bytes: put.size_bytes(),
        },
    );
    let manifest = WorkspaceManifestV1::new(LogicalPath::parse("main.tex").expect("path"), files)
        .expect("manifest");
    let snapshot = manifest.snapshot_id().expect("snapshot");
    let started = Arc::new(AtomicBool::new(false));
    let timer_fired = Arc::new(AtomicBool::new(false));
    let service = CompilerService::new(
        Arc::new(store),
        BlockingRuntime {
            probe: probe(),
            started: Arc::clone(&started),
            timer_fired: Arc::clone(&timer_fired),
        },
        CompilerConfig::new(CompileLimits::development_default()),
    )
    .expect("service");
    let timer = {
        let started = Arc::clone(&started);
        let timer_fired = Arc::clone(&timer_fired);
        tokio::spawn(async move {
            while !started.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            timer_fired.store(true, Ordering::SeqCst);
        })
    };
    service
        .compile(
            snapshot,
            &manifest,
            TexEngine::PdfLatex,
            ShellPolicy::Safe,
            false,
        )
        .await
        .expect("compile with isolated blocking runtime");
    timer.await.expect("timer task");
}

async fn compile_outputs(
    write_output: fn(&std::path::Path),
    exit_code: i32,
    limits: CompileLimits,
) -> Result<compiler::CompileExecution, CompilerError> {
    let directory = tempfile::tempdir().expect("temp");
    let store = FsBlobStore::open(directory.path(), FsBlobStoreConfig::development_default())
        .await
        .expect("store");
    let put = store.put(Bytes::from_static(b"x")).await.expect("put");
    let mut files = BTreeMap::new();
    files.insert(
        LogicalPath::parse("main.tex").expect("path"),
        FileEntryV1 {
            blob_hash: put.hash(),
            size_bytes: put.size_bytes(),
        },
    );
    let manifest = WorkspaceManifestV1::new(LogicalPath::parse("main.tex").expect("path"), files)
        .expect("manifest");
    let snapshot = manifest.snapshot_id().expect("snapshot");
    let service = CompilerService::new(
        Arc::new(store),
        OutputRuntime {
            probe: probe(),
            exit_code,
            write_output,
        },
        CompilerConfig::new(limits),
    )
    .expect("service");
    service
        .compile(
            snapshot,
            &manifest,
            TexEngine::PdfLatex,
            ShellPolicy::Safe,
            false,
        )
        .await
}

fn nested_outputs(output: &std::path::Path) {
    fs::create_dir_all(output.join("chapters")).expect("chapters");
    fs::create_dir_all(output.join("logs")).expect("logs");
    fs::write(output.join("main.pdf"), b"pdf").expect("pdf");
    fs::write(output.join("chapters/ch1.aux"), b"aux").expect("aux");
    fs::write(output.join("logs/build.log"), b"log").expect("log");
}

fn no_outputs(_: &std::path::Path) {}

fn oversized_file(output: &std::path::Path) {
    fs::write(output.join("main.pdf"), b"12345").expect("pdf");
}

fn total_oversized_files(output: &std::path::Path) {
    fs::write(output.join("main.pdf"), b"1234").expect("pdf");
    fs::write(output.join("other.log"), b"5678").expect("log");
}

fn too_many_entries(output: &std::path::Path) {
    for index in 0..8193 {
        fs::create_dir(output.join(format!("d{index}"))).expect("directory");
    }
}

fn too_many_files(output: &std::path::Path) {
    for index in 0..4097 {
        fs::write(output.join(format!("f{index}")), []).expect("file");
    }
}

#[tokio::test]
async fn collects_nested_outputs_in_logical_path_order_with_filename_kinds() {
    let execution = compile_outputs(nested_outputs, 0, CompileLimits::development_default())
        .await
        .expect("compile");
    let artifacts = execution.artifacts();
    assert_eq!(artifacts.len(), 3);
    assert_eq!(artifacts[0].logical_name().as_str(), "chapters/ch1.aux");
    assert_eq!(artifacts[0].kind(), core_types::ArtifactKind::Aux);
    assert_eq!(artifacts[1].logical_name().as_str(), "logs/build.log");
    assert_eq!(artifacts[1].kind(), core_types::ArtifactKind::Log);
    assert_eq!(artifacts[2].logical_name().as_str(), "main.pdf");
    assert_eq!(artifacts[2].kind(), core_types::ArtifactKind::Pdf);
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlinked_output_entries() {
    fn symlink_output(output: &std::path::Path) {
        std::os::unix::fs::symlink("/tmp", output.join("escape")).expect("symlink");
    }
    assert!(matches!(
        compile_outputs(symlink_output, 1, CompileLimits::development_default()).await,
        Err(CompilerError::InvalidOutputEntry { .. })
    ));
}

#[tokio::test]
async fn successful_exit_without_pdf_is_rejected() {
    assert!(matches!(
        compile_outputs(no_outputs, 0, CompileLimits::development_default()).await,
        Err(CompilerError::MissingPdfArtifact)
    ));
}

#[tokio::test]
async fn document_failure_without_pdf_remains_failed() {
    let execution = compile_outputs(no_outputs, 1, CompileLimits::development_default())
        .await
        .expect("failed execution");
    assert_eq!(execution.status(), compiler::CompileStatus::Failed);
}

#[tokio::test]
async fn enforces_artifact_byte_limits() {
    let per_file =
        CompileLimits::new(Duration::from_secs(1), 1, 1, 4, 10, 1, 1, 1.0).expect("limits");
    assert!(matches!(
        compile_outputs(oversized_file, 1, per_file).await,
        Err(CompilerError::ArtifactTooLarge { .. })
    ));
    let total = CompileLimits::new(Duration::from_secs(1), 1, 1, 8, 7, 1, 1, 1.0).expect("limits");
    assert!(matches!(
        compile_outputs(total_oversized_files, 1, total).await,
        Err(CompilerError::TotalArtifactsTooLarge { .. })
    ));
}

#[tokio::test]
async fn enforces_output_entry_and_artifact_file_counts() {
    assert!(matches!(
        compile_outputs(too_many_entries, 1, CompileLimits::development_default()).await,
        Err(CompilerError::OutputEntriesExceeded { limit: 8192 })
    ));
    assert!(matches!(
        compile_outputs(too_many_files, 1, CompileLimits::development_default()).await,
        Err(CompilerError::ArtifactFilesExceeded { limit: 4096 })
    ));
}
