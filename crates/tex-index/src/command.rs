use crate::TexIndexError;
use std::{
    ffi::OsStr,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandResult {
    status_code: Option<i32>,
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl CommandResult {
    #[must_use]
    pub fn new(status_code: Option<i32>, success: bool, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            status_code,
            success,
            stdout,
            stderr,
        }
    }
    #[must_use]
    pub const fn status_code(&self) -> Option<i32> {
        self.status_code
    }
    #[must_use]
    pub const fn success(&self) -> bool {
        self.success
    }
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        program: &Path,
        args: &[&OsStr],
        timeout: Duration,
    ) -> Result<CommandResult, TexIndexError>;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessCommandRunner;
impl CommandRunner for ProcessCommandRunner {
    fn run(
        &self,
        program: &Path,
        args: &[&OsStr],
        timeout: Duration,
    ) -> Result<CommandResult, TexIndexError> {
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| TexIndexError::Io {
                path: program.to_path_buf(),
                source,
            })?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| TexIndexError::InternalInvariant("child stdout was not piped".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| TexIndexError::InternalInvariant("child stderr was not piped".into()))?;
        let stdout_thread = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).map(|_| bytes)
        });
        let stderr_thread = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).map(|_| bytes)
        });
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            TexIndexError::InvalidConfiguration("command timeout is too large".into())
        })?;
        let (status, timed_out) = loop {
            if let Some(status) = child.try_wait().map_err(|source| TexIndexError::Io {
                path: program.to_path_buf(),
                source,
            })? {
                break (status, false);
            }
            if Instant::now() >= deadline {
                child.kill().map_err(|source| TexIndexError::Io {
                    path: program.to_path_buf(),
                    source,
                })?;
                let status = child.wait().map_err(|source| TexIndexError::Io {
                    path: program.to_path_buf(),
                    source,
                })?;
                break (status, true);
            }
            thread::sleep(Duration::from_millis(2));
        };
        let stdout = stdout_thread
            .join()
            .map_err(|_| TexIndexError::InternalInvariant("stdout reader thread failed".into()))?
            .map_err(|source| TexIndexError::Io {
                path: program.to_path_buf(),
                source,
            })?;
        let stderr = stderr_thread
            .join()
            .map_err(|_| TexIndexError::InternalInvariant("stderr reader thread failed".into()))?
            .map_err(|source| TexIndexError::Io {
                path: program.to_path_buf(),
                source,
            })?;
        if timed_out {
            Err(TexIndexError::CommandTimedOut {
                program: program.display().to_string(),
            })
        } else {
            Ok(CommandResult::new(
                status.code(),
                status.success(),
                stdout,
                stderr,
            ))
        }
    }
}
