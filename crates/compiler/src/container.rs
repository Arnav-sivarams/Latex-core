use crate::{CompileLimits, CompilerError};
use bytes::Bytes;
use core_types::{LogicalPath, ShellPolicy, TexEngine};
use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct CompileContainerRequest {
    workspace: PathBuf,
    main_file: LogicalPath,
    engine: TexEngine,
    shell_policy: ShellPolicy,
    synctex: bool,
    limits: CompileLimits,
    execution_id: String,
}

impl CompileContainerRequest {
    #[must_use]
    pub fn new(
        workspace: PathBuf,
        main_file: LogicalPath,
        engine: TexEngine,
        shell_policy: ShellPolicy,
        synctex: bool,
        limits: CompileLimits,
    ) -> Self {
        Self {
            workspace,
            main_file,
            engine,
            shell_policy,
            synctex,
            limits,
            execution_id: "adhoc".to_owned(),
        }
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    #[must_use]
    pub const fn main_file(&self) -> &LogicalPath {
        &self.main_file
    }

    #[must_use]
    pub const fn engine(&self) -> TexEngine {
        self.engine
    }

    #[must_use]
    pub const fn shell_policy(&self) -> ShellPolicy {
        self.shell_policy
    }

    #[must_use]
    pub const fn synctex(&self) -> bool {
        self.synctex
    }

    #[must_use]
    pub const fn limits(&self) -> &CompileLimits {
        &self.limits
    }
    #[must_use]
    pub fn with_execution_id(mut self, execution_id: impl Into<String>) -> Self {
        self.execution_id = execution_id.into();
        self
    }
    #[must_use]
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }
}

#[derive(Clone, Debug)]
pub struct ContainerOutput {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: Bytes,
    pub stderr: Bytes,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

pub trait ContainerRuntime: Send + Sync + 'static {
    fn probe_image(&self) -> Result<Vec<u8>, CompilerError>;
    fn execute(&self, request: &CompileContainerRequest) -> Result<ContainerOutput, CompilerError>;
}

#[derive(Clone, Debug)]
pub struct DockerCliRuntime {
    image: String,
}
impl DockerCliRuntime {
    pub fn new(image: impl Into<String>) -> Result<Self, CompilerError> {
        let image = image.into();
        if !immutable_image(&image) {
            return Err(CompilerError::InvalidImageReference { image });
        }
        Ok(Self { image })
    }
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }
    #[must_use]
    pub fn compile_args(&self, request: &CompileContainerRequest, name: &str) -> Vec<OsString> {
        let bind = format!(
            "--mount=type=bind,source={},target=/work",
            request.workspace().display()
        );
        vec![
            "run".into(),
            "--rm".into(),
            format!("--name={name}").into(),
            "--label=latex-core.application=latex-core".into(),
            format!("--label=latex-core.job-id={}", request.execution_id()).into(),
            "--pull=never".into(),
            "--network=none".into(),
            "--read-only".into(),
            "--cap-drop=ALL".into(),
            "--security-opt=no-new-privileges".into(),
            format!("--pids-limit={}", request.limits().pids_limit()).into(),
            format!("--memory={}", request.limits().memory_bytes()).into(),
            format!("--memory-swap={}", request.limits().memory_bytes()).into(),
            format!("--cpus={}", request.limits().cpu_count()).into(),
            "--user=10001:10001".into(),
            "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size=256m,mode=1777".into(),
            "--env=HOME=/tmp/home".into(),
            "--env=TEXMFHOME=/tmp/texmf-home".into(),
            "--env=TEXMFVAR=/tmp/texmf-var".into(),
            "--env=TEXMFCONFIG=/tmp/texmf-config".into(),
            bind.into(),
            self.image.clone().into(),
            "--engine".into(),
            request.engine().to_string().into(),
            "--shell-policy".into(),
            request.shell_policy().to_string().into(),
            "--synctex".into(),
            if request.synctex() { "1" } else { "0" }.into(),
            "--main".into(),
            request.main_file().as_str().into(),
        ]
    }
}
impl ContainerRuntime for DockerCliRuntime {
    fn probe_image(&self) -> Result<Vec<u8>, CompilerError> {
        ensure_docker_healthy()?;
        let inspection = docker_command()
            .args(["image", "inspect", "--format={{.Id}}", &self.image])
            .output()
            .map_err(|source| CompilerError::DockerUnavailable {
                message: source.to_string(),
            })?;
        if !inspection.status.success() {
            ensure_docker_healthy()?;
            return Err(CompilerError::ImageNotFound {
                image: self.image.clone(),
            });
        }
        let output = docker_command()
            .args([
                "run",
                "--rm",
                "--pull=never",
                "--network=none",
                "--read-only",
                "--cap-drop=ALL",
                "--security-opt=no-new-privileges",
                "--user=10001:10001",
                "--entrypoint=/bin/cat",
                &self.image,
                "/opt/latex-core/tex-environment-index.json",
            ])
            .output()
            .map_err(|source| CompilerError::DockerUnavailable {
                message: source.to_string(),
            })?;
        if !output.status.success() {
            return Err(CompilerError::DockerCommandFailed {
                message: format!(
                    "image probe failed with {}: {}",
                    exit_description(output.status),
                    bounded_diagnostic(&output.stderr)
                ),
            });
        }
        Ok(output.stdout)
    }
    fn execute(&self, request: &CompileContainerRequest) -> Result<ContainerOutput, CompilerError> {
        let name = unique_name(request.workspace());
        let args = self.compile_args(request, &name);
        tracing::info!(
            execution_id = request.execution_id(),
            container_name = %name,
            image = %self.image,
            "starting hardened compiler container"
        );
        let mut child = docker_command()
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| CompilerError::DockerUnavailable {
                message: source.to_string(),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CompilerError::InternalInvariant {
                message: "Docker stdout not piped".into(),
            })?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CompilerError::InternalInvariant {
                message: "Docker stderr not piped".into(),
            })?;
        let stdout_reader = bounded_reader(stdout, request.limits().max_stdout_bytes());
        let stderr_reader = bounded_reader(stderr, request.limits().max_stderr_bytes());
        let deadline = Instant::now()
            .checked_add(request.limits().wall_timeout())
            .ok_or_else(|| CompilerError::InvalidConfiguration {
                message: "timeout overflow".into(),
            })?;
        let (status, timed_out) = wait_for_docker(&mut child, deadline, &name)?;
        let (stdout, stdout_truncated) = join_reader(stdout_reader)?;
        let (stderr, stderr_truncated) = join_reader(stderr_reader)?;
        tracing::info!(
            execution_id = request.execution_id(),
            container_name = %name,
            exit_code = ?status.code(),
            timed_out,
            "hardened compiler container finished"
        );
        if !timed_out {
            match status.code() {
                Some(125) => {
                    return Err(CompilerError::DockerCommandFailed {
                        message: format!(
                            "docker run failed with 125: {}",
                            bounded_diagnostic(&stderr)
                        ),
                    });
                }
                Some(126 | 127) => {
                    return Err(CompilerError::ContainerInfrastructure {
                        message: format!(
                            "docker run infrastructure failure with {}: {}",
                            exit_description(status),
                            bounded_diagnostic(&stderr)
                        ),
                    });
                }
                _ => {}
            }
        }
        Ok(ContainerOutput {
            exit_code: status.code(),
            timed_out,
            stdout: Bytes::from(stdout),
            stderr: Bytes::from(stderr),
            stdout_truncated,
            stderr_truncated,
        })
    }
}

impl DockerCliRuntime {
    /// Reaps only exited/running compiler containers bearing this application's fixed label.
    pub fn reap_orphans(&self) -> Result<u64, CompilerError> {
        ensure_docker_healthy()?;
        let output = docker_command()
            .args([
                "ps",
                "-aq",
                "--filter",
                "label=latex-core.application=latex-core",
            ])
            .output()
            .map_err(|source| CompilerError::DockerUnavailable {
                message: source.to_string(),
            })?;
        if !output.status.success() {
            return Err(CompilerError::DockerCommandFailed {
                message: bounded_diagnostic(&output.stderr),
            });
        }
        let ids = String::from_utf8_lossy(&output.stdout);
        let mut count = 0_u64;
        for id in ids.lines().filter(|id| !id.is_empty()) {
            let result = docker_command()
                .args(["rm", "-f", id])
                .output()
                .map_err(|source| CompilerError::DockerUnavailable {
                    message: source.to_string(),
                })?;
            if !result.status.success() {
                return Err(CompilerError::DockerCommandFailed {
                    message: format!(
                        "cannot reap labelled compiler container {id}: {}",
                        bounded_diagnostic(&result.stderr)
                    ),
                });
            }
            count = count
                .checked_add(1)
                .ok_or_else(|| CompilerError::InternalInvariant {
                    message: "orphan cleanup counter overflow".to_owned(),
                })?;
        }
        Ok(count)
    }
}

fn ensure_docker_healthy() -> Result<(), CompilerError> {
    let output = docker_command().arg("info").output().map_err(|source| {
        CompilerError::DockerUnavailable {
            message: source.to_string(),
        }
    })?;
    if !output.status.success() {
        return Err(CompilerError::DockerCommandFailed {
            message: format!(
                "Docker daemon health check failed with {}: {}",
                exit_description(output.status),
                bounded_diagnostic(&output.stderr)
            ),
        });
    }
    Ok(())
}

fn wait_for_docker(
    child: &mut Child,
    deadline: Instant,
    name: &str,
) -> Result<(ExitStatus, bool), CompilerError> {
    loop {
        if let Some(status) = child.try_wait().map_err(|source| CompilerError::Io {
            operation: "wait for Docker",
            source,
        })? {
            return Ok((status, false));
        }
        if Instant::now() >= deadline {
            return stop_timed_out_container(child, name);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn stop_timed_out_container(
    child: &mut Child,
    name: &str,
) -> Result<(ExitStatus, bool), CompilerError> {
    if let Some(status) = try_wait_after_timeout(child, "perform final Docker wait")? {
        return Ok((status, false));
    }
    match docker_cleanup("stop", name) {
        Ok(stop) if stop.status.success() => Ok((reap_after_timeout(child)?, true)),
        stop => reconcile_or_kill_timed_out_container(child, name, &stop),
    }
}

fn reconcile_or_kill_timed_out_container(
    child: &mut Child,
    name: &str,
    stop: &Result<Output, CompilerError>,
) -> Result<(ExitStatus, bool), CompilerError> {
    if let Some(status) = try_wait_after_timeout(child, "reconcile Docker after stop")? {
        return Ok((status, false));
    }
    match docker_cleanup("kill", name) {
        Ok(kill) if kill.status.success() => Ok((reap_after_timeout(child)?, true)),
        kill => {
            if let Some(status) = try_wait_after_timeout(child, "reconcile Docker after kill")? {
                return Ok((status, false));
            }
            Err(CompilerError::ContainerInfrastructure {
                message: format!(
                    "could not terminate timed-out container {name}: stop {}; kill {}",
                    cleanup_description(stop),
                    cleanup_description(&kill)
                ),
            })
        }
    }
}

fn try_wait_after_timeout(
    child: &mut Child,
    operation: &'static str,
) -> Result<Option<ExitStatus>, CompilerError> {
    child
        .try_wait()
        .map_err(|source| CompilerError::ContainerInfrastructure {
            message: format!("cannot {operation}: {source}"),
        })
}

fn docker_command() -> Command {
    let mut command = Command::new("docker");
    command.env_clear().env("PATH", "/usr/bin:/bin");
    command
}

fn docker_cleanup(action: &str, name: &str) -> Result<Output, CompilerError> {
    let arguments = match action {
        "stop" => vec!["stop", "--time=2", name],
        "kill" => vec!["kill", name],
        _ => {
            return Err(CompilerError::InternalInvariant {
                message: "invalid Docker cleanup action".into(),
            });
        }
    };
    docker_command().args(arguments).output().map_err(|source| {
        CompilerError::ContainerInfrastructure {
            message: format!("cannot run Docker {action} for {name}: {source}"),
        }
    })
}

fn reap_after_timeout(child: &mut Child) -> Result<ExitStatus, CompilerError> {
    child
        .wait()
        .map_err(|source| CompilerError::ContainerInfrastructure {
            message: format!("cannot reap Docker after timeout: {source}"),
        })
}

fn cleanup_description(result: &Result<Output, CompilerError>) -> String {
    match result {
        Ok(output) => format!(
            "{} ({})",
            exit_description(output.status),
            bounded_diagnostic(&output.stderr)
        ),
        Err(error) => error.to_string(),
    }
}

fn exit_description(status: ExitStatus) -> String {
    status
        .code()
        .map_or_else(|| "signal".to_owned(), |code| format!("exit code {code}"))
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    const MAX_DIAGNOSTIC_BYTES: usize = 8 * 1024;
    let bounded = &bytes[..bytes.len().min(MAX_DIAGNOSTIC_BYTES)];
    let mut text = String::from_utf8_lossy(bounded).into_owned();
    if bytes.len() > bounded.len() {
        text.push_str(" [truncated]");
    }
    text
}

fn bounded_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> thread::JoinHandle<std::io::Result<(Vec<u8>, bool)>> {
    thread::spawn(move || {
        let mut result = Vec::with_capacity(limit.min(8192));
        let mut buffer = [0_u8; 8192];
        let mut truncated = false;
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let room = limit.saturating_sub(result.len());
            let take = room.min(count);
            result.extend_from_slice(&buffer[..take]);
            truncated |= take < count;
        }
        Ok((result, truncated))
    })
}
fn join_reader(
    handle: thread::JoinHandle<std::io::Result<(Vec<u8>, bool)>>,
) -> Result<(Vec<u8>, bool), CompilerError> {
    handle
        .join()
        .map_err(|_| CompilerError::InternalInvariant {
            message: "output reader panicked".into(),
        })?
        .map_err(|source| CompilerError::Io {
            operation: "read Docker output",
            source,
        })
}
fn immutable_image(value: &str) -> bool {
    let digest = value.strip_prefix("sha256:").or_else(|| {
        value
            .split_once("@sha256:")
            .and_then(|(repository, digest)| valid_repository(repository).then_some(digest))
    });
    digest.is_some_and(valid_sha256_digest)
}

fn valid_repository(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('@')
        && !value.bytes().any(|byte| byte.is_ascii_whitespace())
}

fn valid_sha256_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
fn unique_name(workspace: &Path) -> String {
    let suffix = workspace
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("workspace")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    format!("latex-core-m7-{}-{suffix}", std::process::id())
}
