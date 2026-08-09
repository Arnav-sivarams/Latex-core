use crate::{CommandRunner, TexIndexError};
use std::{ffi::OsStr, path::Path, time::Duration};

pub(crate) fn run_text(
    runner: &dyn CommandRunner,
    program: &Path,
    args: &[&OsStr],
    timeout: Duration,
) -> Result<String, TexIndexError> {
    let result = runner.run(program, args, timeout)?;
    if !result.success() {
        return Err(TexIndexError::CommandFailed {
            program: program.display().to_string(),
            status: result.status_code(),
            stderr: String::from_utf8_lossy(result.stderr()).into_owned(),
        });
    }
    String::from_utf8(result.stdout().to_vec())
        .map(|s| s.trim().to_owned())
        .map_err(|e| TexIndexError::InvalidCommandOutput {
            program: program.display().to_string(),
            message: e.to_string(),
        })
}

pub(crate) fn detect_year(text: &str) -> Option<u16> {
    for token in text.split(|c: char| !c.is_ascii_digit()) {
        if token.len() == 4 {
            if let Ok(year) = token.parse::<u16>() {
                if (2000..=2999).contains(&year) {
                    return Some(year);
                }
            }
        }
    }
    None
}
