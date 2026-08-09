use crate::{
    IndexedTexFile, TexConfigRecord, TexEnvironmentConfig, TexEnvironmentIndexV1, TexIndexError,
    TexLivePackage, TexLiveRelease, TexToolKind, TexToolRecord,
    command::CommandRunner,
    probe::{detect_year, run_text},
    tlpdb::parse_tlpdb,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct TexIndexBuilder {
    config: TexEnvironmentConfig,
    runner: Arc<dyn CommandRunner>,
}
impl std::fmt::Debug for TexIndexBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TexIndexBuilder")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
impl TexIndexBuilder {
    #[must_use]
    pub fn new(config: TexEnvironmentConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self { config, runner }
    }
    #[allow(
        clippy::too_many_lines,
        reason = "the frozen build flow is intentionally linear"
    )]
    pub fn build(&self) -> Result<TexEnvironmentIndexV1, TexIndexError> {
        let executables = self.executables()?;
        let kpse = executables.get(&TexToolKind::Kpsewhich).ok_or_else(|| {
            TexIndexError::InternalInvariant("kpsewhich absent after validation".into())
        })?;
        let tlmgr = executables.get(&TexToolKind::Tlmgr).ok_or_else(|| {
            TexIndexError::InternalInvariant("tlmgr absent after validation".into())
        })?;
        let tlmgr_version = run_text(
            self.runner.as_ref(),
            tlmgr,
            &[OsStr::new("--version")],
            self.config.command_timeout(),
        )?;
        let kpse_version = run_text(
            self.runner.as_ref(),
            kpse,
            &[OsStr::new("--version")],
            self.config.command_timeout(),
        )?;
        let actual = detect_year(&tlmgr_version)
            .or_else(|| detect_year(&kpse_version))
            .ok_or_else(|| TexIndexError::InvalidCommandOutput {
                program: tlmgr.display().to_string(),
                message: "TeX Live release year not found".into(),
            })?;
        if actual != self.config.required_release_year() {
            return Err(TexIndexError::WrongTexLiveRelease {
                required: self.config.required_release_year(),
                actual,
            });
        }
        let root_text = run_text(
            self.runner.as_ref(),
            kpse,
            &[OsStr::new("-var-value=TEXMFROOT")],
            self.config.command_timeout(),
        )?;
        let root = PathBuf::from(root_text);
        if !root.is_dir() {
            return Err(TexIndexError::MissingTexmfRoot(root));
        }
        let canonical_root = fs::canonicalize(&root).map_err(|source| TexIndexError::Io {
            path: root.clone(),
            source,
        })?;
        let tlpdb = root.join("tlpkg/texlive.tlpdb");
        if !tlpdb.is_file() {
            return Err(TexIndexError::MissingTlpdb(tlpdb));
        }
        let db = fs::read_to_string(&tlpdb).map_err(|source| TexIndexError::Io {
            path: tlpdb.clone(),
            source,
        })?;
        let records = parse_tlpdb(&db)?;
        let platform_path = run_text(
            self.runner.as_ref(),
            kpse,
            &[OsStr::new("-var-value=SELFAUTODIR")],
            self.config.command_timeout(),
        )?;
        let platform = Path::new(&platform_path)
            .file_name()
            .and_then(OsStr::to_str)
            .filter(|s| !s.is_empty() && s.is_ascii())
            .ok_or_else(|| TexIndexError::InvalidCommandOutput {
                program: kpse.display().to_string(),
                message: "platform unavailable".into(),
            })?
            .to_owned();
        let release = TexLiveRelease::new(actual, platform)?;
        let mut packages = BTreeMap::new();
        for record in records.values().filter(|r| r.category() == "Package") {
            let mut files = Vec::new();
            for relative in record.runfiles() {
                let physical = safe_file(&canonical_root, relative)?;
                files.push(IndexedTexFile::new(
                    relative.clone(),
                    hash_file(&physical)?,
                )?);
            }
            let package = TexLivePackage::new(
                record.name().to_owned(),
                record.category().to_owned(),
                record.revision(),
                record.catalogue_version().map(str::to_owned),
                record.catalogue_license().map(str::to_owned),
                files,
            )?;
            packages.insert(record.name().to_owned(), package);
        }
        let mut tools = BTreeMap::new();
        for kind in TexToolKind::ALL {
            let path = executables
                .get(&kind)
                .ok_or_else(|| TexIndexError::InternalInvariant("tool missing".into()))?;
            let args = [OsStr::new(if kind == TexToolKind::Makeglossaries {
                "--help"
            } else {
                "--version"
            })];
            let version = if kind == TexToolKind::Kpsewhich {
                kpse_version.clone()
            } else if kind == TexToolKind::Tlmgr {
                tlmgr_version.clone()
            } else {
                run_text(
                    self.runner.as_ref(),
                    path,
                    &args,
                    self.config.command_timeout(),
                )?
            };
            tools.insert(kind, TexToolRecord::new(kind, version, hash_file(path)?)?);
        }
        let mut config_files = BTreeMap::new();
        for name in [
            "texmf.cnf",
            "fmtutil.cnf",
            "updmap.cfg",
            "language.dat",
            "language.def",
            "language.dat.lua",
        ] {
            if let Some(output) = optional_kpse(
                self.runner.as_ref(),
                kpse,
                name,
                self.config.command_timeout(),
            )? {
                let selected =
                    output
                        .lines()
                        .next()
                        .ok_or_else(|| TexIndexError::InvalidCommandOutput {
                            program: kpse.display().to_string(),
                            message: "empty config result".into(),
                        })?;
                let physical = PathBuf::from(selected);
                if !physical.is_file() {
                    return Err(TexIndexError::InvalidRuntimeFile(name.into()));
                }
                let canonical =
                    fs::canonicalize(&physical).map_err(|source| TexIndexError::Io {
                        path: physical.clone(),
                        source,
                    })?;
                if !canonical.starts_with(&canonical_root) {
                    return Err(TexIndexError::StorageEntryEscape(name.into()));
                }
                let record = TexConfigRecord::new(name.into(), hash_file(&canonical)?)?;
                config_files.insert(name.into(), record);
            }
        }
        TexEnvironmentIndexV1::new(release, packages, tools, config_files)
    }
    fn executables(&self) -> Result<BTreeMap<TexToolKind, PathBuf>, TexIndexError> {
        let mut map = BTreeMap::new();
        for kind in TexToolKind::ALL {
            let raw = self.config.bin_dir().join(kind.basename());
            if !raw.is_file() {
                return Err(TexIndexError::MissingExecutable(kind.basename().into()));
            }
            let path = fs::canonicalize(&raw).map_err(|source| TexIndexError::Io {
                path: raw.clone(),
                source,
            })?;
            map.insert(kind, path);
        }
        Ok(map)
    }
}
fn optional_kpse(
    runner: &dyn CommandRunner,
    program: &Path,
    name: &str,
    timeout: std::time::Duration,
) -> Result<Option<String>, TexIndexError> {
    let result = runner.run(program, &[OsStr::new(name)], timeout)?;
    if !result.success() {
        if result.stdout().is_empty() {
            return Ok(None);
        }
        return Err(TexIndexError::CommandFailed {
            program: program.display().to_string(),
            status: result.status_code(),
            stderr: String::from_utf8_lossy(result.stderr()).into_owned(),
        });
    }
    let text = String::from_utf8(result.stdout().to_vec()).map_err(|error| {
        TexIndexError::InvalidCommandOutput {
            program: program.display().to_string(),
            message: error.to_string(),
        }
    })?;
    let text = text.trim();
    Ok((!text.is_empty()).then(|| text.to_owned()))
}
fn safe_file(root: &Path, relative: &str) -> Result<PathBuf, TexIndexError> {
    let joined = root.join(relative);
    if !joined.exists() {
        return Err(TexIndexError::MissingRuntimeFile(relative.into()));
    }
    let canonical = fs::canonicalize(&joined).map_err(|source| TexIndexError::Io {
        path: joined.clone(),
        source,
    })?;
    if !canonical.starts_with(root) {
        return Err(TexIndexError::StorageEntryEscape(relative.into()));
    }
    if !canonical.is_file() {
        return Err(TexIndexError::InvalidRuntimeFile(relative.into()));
    }
    Ok(canonical)
}
pub(crate) fn hash_file(path: &Path) -> Result<String, TexIndexError> {
    let bytes = fs::read(path).map_err(|source| TexIndexError::HashFailure {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
