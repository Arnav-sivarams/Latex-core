use crate::{CommandRunner, TexEnvironmentConfig, TexIndexError, TexResolvedFile};
use std::{ffi::OsStr, path::PathBuf, sync::Arc};

/// Kpathsea-backed runtime resolver. Static index ordering is not Kpathsea precedence.
#[derive(Clone)]
pub struct KpathseaResolver {
    config: TexEnvironmentConfig,
    runner: Arc<dyn CommandRunner>,
}
impl std::fmt::Debug for KpathseaResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KpathseaResolver")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
impl KpathseaResolver {
    #[must_use]
    pub fn new(config: TexEnvironmentConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self { config, runner }
    }
    pub fn resolve(&self, filename: &str) -> Result<Option<TexResolvedFile>, TexIndexError> {
        Ok(self.resolve_impl(filename, false)?.into_iter().next())
    }
    pub fn resolve_all(&self, filename: &str) -> Result<Vec<TexResolvedFile>, TexIndexError> {
        self.resolve_impl(filename, true)
    }
    fn resolve_impl(
        &self,
        filename: &str,
        all: bool,
    ) -> Result<Vec<TexResolvedFile>, TexIndexError> {
        validate_lookup(filename)?;
        let program = self.config.bin_dir().join("kpsewhich");
        let all_arg = OsStr::new("--all");
        let name = OsStr::new(filename);
        let args: Vec<&OsStr> = if all { vec![all_arg, name] } else { vec![name] };
        let result = self
            .runner
            .run(&program, &args, self.config.command_timeout())?;
        if !result.success() && result.stdout().is_empty() {
            return Ok(Vec::new());
        }
        if !result.success() {
            return Err(TexIndexError::CommandFailed {
                program: program.display().to_string(),
                status: result.status_code(),
                stderr: String::from_utf8_lossy(result.stderr()).into_owned(),
            });
        }
        let text = String::from_utf8(result.stdout().to_vec())
            .map_err(|error| TexIndexError::InvalidCommandOutput {
                program: program.display().to_string(),
                message: error.to_string(),
            })?
            .trim()
            .to_owned();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        Ok(text
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| TexResolvedFile::new(filename.to_owned(), PathBuf::from(line)))
            .collect())
    }
}
fn validate_lookup(value: &str) -> Result<(), TexIndexError> {
    if value.is_empty() || value.starts_with('-') || value.contains(['\0', '\n', '\r']) {
        return Err(TexIndexError::InvalidConfiguration(
            "invalid Kpathsea lookup name".into(),
        ));
    }
    Ok(())
}
