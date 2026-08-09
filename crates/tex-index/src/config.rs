use crate::TexIndexError;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct TexEnvironmentConfig {
    bin_dir: PathBuf,
    required_release_year: u16,
    command_timeout: Duration,
}
impl TexEnvironmentConfig {
    pub fn new(
        bin_dir: PathBuf,
        required_release_year: u16,
        command_timeout: Duration,
    ) -> Result<Self, TexIndexError> {
        if !bin_dir.is_dir() {
            return Err(TexIndexError::InvalidConfiguration(format!(
                "bin_dir is not a directory: {}",
                bin_dir.display()
            )));
        }
        if required_release_year == 0 {
            return Err(TexIndexError::InvalidConfiguration(
                "release year must be nonzero".into(),
            ));
        }
        if command_timeout.is_zero() {
            return Err(TexIndexError::InvalidConfiguration(
                "command timeout must be nonzero".into(),
            ));
        }
        Ok(Self {
            bin_dir,
            required_release_year,
            command_timeout,
        })
    }
    pub fn new_2026(bin_dir: PathBuf) -> Result<Self, TexIndexError> {
        Self::new(bin_dir, 2026, Duration::from_secs(30))
    }
    #[must_use]
    pub fn bin_dir(&self) -> &Path {
        &self.bin_dir
    }
    #[must_use]
    pub const fn required_release_year(&self) -> u16 {
        self.required_release_year
    }
    #[must_use]
    pub const fn command_timeout(&self) -> Duration {
        self.command_timeout
    }
}
