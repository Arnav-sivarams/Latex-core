use crate::CompilerError;
use bytes::Bytes;
use core_types::{ArtifactKind, CompileKey, LatexmkProfileId, LogicalPath, TexEnvironmentId};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CompileStatus {
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompilerArtifact {
    kind: ArtifactKind,
    logical_name: LogicalPath,
    bytes: Bytes,
}
impl CompilerArtifact {
    pub(crate) fn new(kind: ArtifactKind, logical_name: LogicalPath, bytes: Bytes) -> Self {
        Self {
            kind,
            logical_name,
            bytes,
        }
    }
    #[must_use]
    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }
    #[must_use]
    pub const fn logical_name(&self) -> &LogicalPath {
        &self.logical_name
    }
    #[must_use]
    pub const fn bytes(&self) -> &Bytes {
        &self.bytes
    }
}

#[derive(Clone, Debug)]
pub struct CompileExecution {
    status: CompileStatus,
    compile_key: CompileKey,
    environment_id: TexEnvironmentId,
    latexmk_profile: LatexmkProfileId,
    artifacts: Vec<CompilerArtifact>,
    stdout: Bytes,
    stderr: Bytes,
    stdout_truncated: bool,
    stderr_truncated: bool,
    exit_code: Option<i32>,
}
impl CompileExecution {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        status: CompileStatus,
        compile_key: CompileKey,
        environment_id: TexEnvironmentId,
        latexmk_profile: LatexmkProfileId,
        artifacts: Vec<CompilerArtifact>,
        stdout: Bytes,
        stderr: Bytes,
        stdout_truncated: bool,
        stderr_truncated: bool,
        exit_code: Option<i32>,
    ) -> Self {
        Self {
            status,
            compile_key,
            environment_id,
            latexmk_profile,
            artifacts,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
            exit_code,
        }
    }
    #[must_use]
    pub const fn status(&self) -> CompileStatus {
        self.status
    }
    #[must_use]
    pub const fn compile_key(&self) -> CompileKey {
        self.compile_key
    }
    #[must_use]
    pub const fn environment_id(&self) -> &TexEnvironmentId {
        &self.environment_id
    }
    #[must_use]
    pub const fn latexmk_profile(&self) -> &LatexmkProfileId {
        &self.latexmk_profile
    }
    #[must_use]
    pub fn artifacts(&self) -> &[CompilerArtifact] {
        &self.artifacts
    }
    #[must_use]
    pub const fn stdout(&self) -> &Bytes {
        &self.stdout
    }
    #[must_use]
    pub const fn stderr(&self) -> &Bytes {
        &self.stderr
    }
    #[must_use]
    pub const fn stdout_truncated(&self) -> bool {
        self.stdout_truncated
    }
    #[must_use]
    pub const fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }
    #[must_use]
    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
}

#[derive(Clone, Debug)]
pub struct CompileLimits {
    wall_timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    max_artifact_bytes: u64,
    max_total_artifact_bytes: u64,
    memory_bytes: u64,
    pids_limit: u32,
    cpu_count: f64,
}
impl CompileLimits {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        wall_timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
        max_artifact_bytes: u64,
        max_total_artifact_bytes: u64,
        memory_bytes: u64,
        pids_limit: u32,
        cpu_count: f64,
    ) -> Result<Self, CompilerError> {
        if wall_timeout.is_zero()
            || max_stdout_bytes == 0
            || max_stderr_bytes == 0
            || max_artifact_bytes == 0
            || max_total_artifact_bytes == 0
            || memory_bytes == 0
            || pids_limit == 0
            || !cpu_count.is_finite()
            || cpu_count <= 0.0
        {
            return Err(CompilerError::InvalidConfiguration {
                message: "all compile limits must be positive and finite".into(),
            });
        }
        Ok(Self {
            wall_timeout,
            max_stdout_bytes,
            max_stderr_bytes,
            max_artifact_bytes,
            max_total_artifact_bytes,
            memory_bytes,
            pids_limit,
            cpu_count,
        })
    }
    #[must_use]
    pub fn development_default() -> Self {
        Self {
            wall_timeout: Duration::from_secs(60),
            max_stdout_bytes: 4 * 1024 * 1024,
            max_stderr_bytes: 4 * 1024 * 1024,
            max_artifact_bytes: 128 * 1024 * 1024,
            max_total_artifact_bytes: 256 * 1024 * 1024,
            memory_bytes: 1024 * 1024 * 1024,
            pids_limit: 128,
            cpu_count: 1.0,
        }
    }
    #[must_use]
    pub const fn wall_timeout(&self) -> Duration {
        self.wall_timeout
    }
    #[must_use]
    pub const fn max_stdout_bytes(&self) -> usize {
        self.max_stdout_bytes
    }
    #[must_use]
    pub const fn max_stderr_bytes(&self) -> usize {
        self.max_stderr_bytes
    }
    #[must_use]
    pub const fn max_artifact_bytes(&self) -> u64 {
        self.max_artifact_bytes
    }
    #[must_use]
    pub const fn max_total_artifact_bytes(&self) -> u64 {
        self.max_total_artifact_bytes
    }
    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }
    #[must_use]
    pub const fn pids_limit(&self) -> u32 {
        self.pids_limit
    }
    #[must_use]
    pub const fn cpu_count(&self) -> f64 {
        self.cpu_count
    }
}
