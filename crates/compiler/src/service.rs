use crate::{
    CompileContainerRequest, CompileExecution, CompileLimits, CompileStatus, CompilerArtifact,
    CompilerError, ContainerRuntime, materialize::materialize, profile_for,
};
use blob_store::BlobStore;
use bytes::Bytes;
use core_types::{
    ArtifactKind, CompileKeyMaterialV1, LogicalPath, ShellPolicy, SnapshotId, TexEngine,
    TexEnvironmentId, WorkspaceManifestV1,
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tex_index::TexEnvironmentIndexV1;

const MAX_OUTPUT_ENTRIES: usize = 8192;
const MAX_ARTIFACT_FILES: usize = 4096;

#[derive(Clone, Debug)]
pub struct CompilerConfig {
    limits: CompileLimits,
    staging_root: Option<PathBuf>,
}
impl CompilerConfig {
    #[must_use]
    pub fn new(limits: CompileLimits) -> Self {
        Self {
            limits,
            staging_root: None,
        }
    }
    #[must_use]
    pub const fn limits(&self) -> &CompileLimits {
        &self.limits
    }
    #[must_use]
    pub fn with_staging_root(mut self, root: PathBuf) -> Self {
        self.staging_root = Some(root);
        self
    }
}

pub struct CompilerService<R: ContainerRuntime> {
    blobs: Arc<dyn BlobStore>,
    runtime: Arc<R>,
    config: CompilerConfig,
    index: TexEnvironmentIndexV1,
    environment_id: TexEnvironmentId,
}
impl<R: ContainerRuntime> std::fmt::Debug for CompilerService<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilerService")
            .field("config", &self.config)
            .field("environment_id", &self.environment_id)
            .finish_non_exhaustive()
    }
}
impl<R: ContainerRuntime> CompilerService<R> {
    pub fn new(
        blobs: Arc<dyn BlobStore>,
        runtime: R,
        config: CompilerConfig,
    ) -> Result<Self, CompilerError> {
        let bytes = runtime.probe_image()?;
        let index = TexEnvironmentIndexV1::from_json_bytes(&bytes).map_err(|error| {
            CompilerError::EnvironmentMismatch {
                message: error.to_string(),
            }
        })?;
        if index.release().year() != 2026 {
            return Err(CompilerError::EnvironmentMismatch {
                message: format!("required TeX Live 2026, found {}", index.release().year()),
            });
        }
        let environment_id =
            index
                .environment_id()
                .map_err(|error| CompilerError::EnvironmentMismatch {
                    message: error.to_string(),
                })?;
        Ok(Self {
            blobs,
            runtime: Arc::new(runtime),
            config,
            index,
            environment_id,
        })
    }
    #[must_use]
    pub const fn environment_index(&self) -> &TexEnvironmentIndexV1 {
        &self.index
    }
    #[must_use]
    pub const fn environment_id(&self) -> &TexEnvironmentId {
        &self.environment_id
    }
    pub async fn compile(
        &self,
        snapshot_id: SnapshotId,
        manifest: &WorkspaceManifestV1,
        engine: TexEngine,
        shell_policy: ShellPolicy,
        synctex: bool,
    ) -> Result<CompileExecution, CompilerError> {
        self.compile_with_execution_id(
            snapshot_id,
            manifest,
            engine,
            shell_policy,
            synctex,
            "adhoc",
        )
        .await
    }
    #[allow(
        clippy::too_many_lines,
        reason = "compile execution owns one cleanup boundary"
    )]
    pub async fn compile_with_execution_id(
        &self,
        snapshot_id: SnapshotId,
        manifest: &WorkspaceManifestV1,
        engine: TexEngine,
        shell_policy: ShellPolicy,
        synctex: bool,
        execution_id: &str,
    ) -> Result<CompileExecution, CompilerError> {
        let profile = profile_for(shell_policy)?;
        let actual = manifest
            .snapshot_id()
            .map_err(|error| CompilerError::InternalInvariant {
                message: error.to_string(),
            })?;
        if actual != snapshot_id {
            return Err(CompilerError::SnapshotMismatch {
                requested: snapshot_id,
                actual,
            });
        }
        let key = CompileKeyMaterialV1::new(
            snapshot_id,
            engine,
            self.environment_id.clone(),
            profile.clone(),
            shell_policy,
            synctex,
        )
        .compile_key()
        .map_err(|error| CompilerError::InternalInvariant {
            message: error.to_string(),
        })?;
        let mut builder = tempfile::Builder::new();
        let builder = builder.prefix("latex-core-compile-");
        let temporary = match &self.config.staging_root {
            Some(root) => {
                fs::create_dir_all(root).map_err(|source| CompilerError::Io {
                    operation: "create worker staging root",
                    source,
                })?;
                builder.tempdir_in(root)
            }
            None => builder.tempdir(),
        }
        .map_err(|source| CompilerError::Io {
            operation: "create compile workspace",
            source,
        })?;
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o755)).map_err(
            |source| CompilerError::Io {
                operation: "set workspace permissions",
                source,
            },
        )?;
        materialize(temporary.path(), snapshot_id, manifest, &self.blobs).await?;
        fs::create_dir(temporary.path().join(".latex-core-out")).map_err(|source| {
            CompilerError::Io {
                operation: "create output directory",
                source,
            }
        })?;
        fs::set_permissions(
            temporary.path().join(".latex-core-out"),
            fs::Permissions::from_mode(0o777),
        )
        .map_err(|source| CompilerError::Io {
            operation: "set output permissions",
            source,
        })?;
        let request = CompileContainerRequest::new(
            temporary.path().to_path_buf(),
            manifest.main_file().clone(),
            engine,
            shell_policy,
            synctex,
            self.config.limits().clone(),
        )
        .with_execution_id(execution_id);
        let runtime = Arc::clone(&self.runtime);
        let output = tokio::task::spawn_blocking(move || runtime.execute(&request))
            .await
            .map_err(|source| CompilerError::BlockingTaskFailed { source })??;
        let artifacts = collect_artifacts(temporary.path(), self.config.limits())?;
        if !output.timed_out
            && output.exit_code == Some(0)
            && !artifacts
                .iter()
                .any(|artifact| artifact.kind() == ArtifactKind::Pdf)
        {
            return Err(CompilerError::MissingPdfArtifact);
        }
        let status = if output.timed_out {
            CompileStatus::TimedOut
        } else if output.exit_code == Some(0) {
            CompileStatus::Succeeded
        } else {
            CompileStatus::Failed
        };
        Ok(CompileExecution::new(
            status,
            key,
            self.environment_id.clone(),
            profile,
            artifacts,
            output.stdout,
            output.stderr,
            output.stdout_truncated,
            output.stderr_truncated,
            output.exit_code,
        ))
    }
}

fn collect_artifacts(
    workspace: &Path,
    limits: &CompileLimits,
) -> Result<Vec<CompilerArtifact>, CompilerError> {
    let output = workspace.join(".latex-core-out");
    let mut directories = vec![output.clone()];
    let mut output_entries = 0_usize;
    let mut artifact_files = 0_usize;
    let mut total = 0_u64;
    let mut artifacts = Vec::new();
    while let Some(directory) = directories.pop() {
        let remaining = MAX_OUTPUT_ENTRIES.checked_sub(output_entries).ok_or(
            CompilerError::OutputEntriesExceeded {
                limit: MAX_OUTPUT_ENTRIES,
            },
        )?;
        let entries = read_output_directory(&directory, remaining)?;
        output_entries = output_entries.checked_add(entries.len()).ok_or(
            CompilerError::OutputEntriesExceeded {
                limit: MAX_OUTPUT_ENTRIES,
            },
        )?;
        for path in entries.into_iter().rev() {
            let metadata = fs::symlink_metadata(&path).map_err(|source| CompilerError::Io {
                operation: "inspect output entry",
                source,
            })?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(CompilerError::InvalidOutputEntry { path });
            }
            if file_type.is_dir() {
                directories.push(path);
                continue;
            }
            if !file_type.is_file() {
                return Err(CompilerError::InvalidOutputEntry { path });
            }
            artifact_files =
                artifact_files
                    .checked_add(1)
                    .ok_or(CompilerError::ArtifactFilesExceeded {
                        limit: MAX_ARTIFACT_FILES,
                    })?;
            if artifact_files > MAX_ARTIFACT_FILES {
                return Err(CompilerError::ArtifactFilesExceeded {
                    limit: MAX_ARTIFACT_FILES,
                });
            }
            collect_artifact(
                &output,
                path,
                metadata.len(),
                limits,
                &mut total,
                &mut artifacts,
            )?;
        }
    }
    artifacts.sort_by(|left, right| left.logical_name().cmp(right.logical_name()));
    Ok(artifacts)
}

fn read_output_directory(
    directory: &Path,
    remaining: usize,
) -> Result<Vec<PathBuf>, CompilerError> {
    let mut entries = Vec::with_capacity(remaining);
    for entry in fs::read_dir(directory).map_err(|source| CompilerError::Io {
        operation: "read output directory",
        source,
    })? {
        let entry = entry.map_err(|source| CompilerError::Io {
            operation: "read output entry",
            source,
        })?;
        if entries.len() == remaining {
            return Err(CompilerError::OutputEntriesExceeded {
                limit: MAX_OUTPUT_ENTRIES,
            });
        }
        entries.push(entry.path());
    }
    entries.sort();
    Ok(entries)
}

fn collect_artifact(
    output: &Path,
    path: PathBuf,
    size: u64,
    limits: &CompileLimits,
    total: &mut u64,
    artifacts: &mut Vec<CompilerArtifact>,
) -> Result<(), CompilerError> {
    if size > limits.max_artifact_bytes() {
        return Err(CompilerError::ArtifactTooLarge { path, size });
    }
    *total = total
        .checked_add(size)
        .ok_or(CompilerError::TotalArtifactsTooLarge { size: u64::MAX })?;
    if *total > limits.max_total_artifact_bytes() {
        return Err(CompilerError::TotalArtifactsTooLarge { size: *total });
    }
    let logical = output_logical_path(output, &path)?;
    let bytes = fs::read(&path).map_err(|source| CompilerError::Io {
        operation: "read artifact",
        source,
    })?;
    let name = logical.file_name();
    artifacts.push(CompilerArtifact::new(
        kind(name),
        logical,
        Bytes::from(bytes),
    ));
    Ok(())
}

fn output_logical_path(output: &Path, path: &Path) -> Result<LogicalPath, CompilerError> {
    let relative = path
        .strip_prefix(output)
        .map_err(|_| CompilerError::InvalidOutputEntry {
            path: path.to_path_buf(),
        })?;
    let mut components = Vec::new();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(CompilerError::InvalidOutputEntry {
                path: path.to_path_buf(),
            });
        };
        let value = value
            .to_str()
            .ok_or_else(|| CompilerError::InvalidOutputEntry {
                path: path.to_path_buf(),
            })?;
        components.push(value);
    }
    LogicalPath::parse(&components.join("/")).map_err(|_| CompilerError::InvalidOutputEntry {
        path: path.to_path_buf(),
    })
}

fn kind(name: &str) -> ArtifactKind {
    if name.ends_with(".synctex.gz") {
        ArtifactKind::Synctex
    } else {
        match Path::new(name).extension().and_then(|value| value.to_str()) {
            Some("pdf") => ArtifactKind::Pdf,
            Some("log") => ArtifactKind::Log,
            Some("fls") => ArtifactKind::Fls,
            Some("aux") => ArtifactKind::Aux,
            Some("bcf") => ArtifactKind::Bcf,
            Some("toc") => ArtifactKind::Toc,
            _ => ArtifactKind::Other,
        }
    }
}
