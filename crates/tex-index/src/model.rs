use crate::TexIndexError;
use core_types::TexEnvironmentId;
use serde::{Deserialize, Deserializer, Serialize, de};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum TexToolKind {
    Kpsewhich,
    Tlmgr,
    PdfLatex,
    LuaLatex,
    XeLatex,
    Bibtex,
    Biber,
    Makeindex,
    Makeglossaries,
    Latexmk,
}
impl TexToolKind {
    pub const ALL: [Self; 10] = [
        Self::Kpsewhich,
        Self::Tlmgr,
        Self::PdfLatex,
        Self::LuaLatex,
        Self::XeLatex,
        Self::Bibtex,
        Self::Biber,
        Self::Makeindex,
        Self::Makeglossaries,
        Self::Latexmk,
    ];
    #[must_use]
    pub const fn basename(self) -> &'static str {
        match self {
            Self::Kpsewhich => "kpsewhich",
            Self::Tlmgr => "tlmgr",
            Self::PdfLatex => "pdflatex",
            Self::LuaLatex => "lualatex",
            Self::XeLatex => "xelatex",
            Self::Bibtex => "bibtex",
            Self::Biber => "biber",
            Self::Makeindex => "makeindex",
            Self::Makeglossaries => "makeglossaries",
            Self::Latexmk => "latexmk",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TexToolRecord {
    kind: TexToolKind,
    version_output: String,
    binary_sha256: String,
}
impl TexToolRecord {
    pub fn new(
        kind: TexToolKind,
        version_output: String,
        binary_sha256: String,
    ) -> Result<Self, TexIndexError> {
        validate_hash(&binary_sha256)?;
        if version_output.is_empty() {
            return Err(TexIndexError::InternalInvariant(
                "empty tool version".into(),
            ));
        }
        Ok(Self {
            kind,
            version_output,
            binary_sha256,
        })
    }
    #[must_use]
    pub const fn kind(&self) -> TexToolKind {
        self.kind
    }
    #[must_use]
    pub fn version_output(&self) -> &str {
        &self.version_output
    }
    #[must_use]
    pub fn binary_sha256(&self) -> &str {
        &self.binary_sha256
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TexLiveRelease {
    year: u16,
    platform: String,
}
impl TexLiveRelease {
    pub fn new(year: u16, platform: String) -> Result<Self, TexIndexError> {
        if year == 0 || platform.is_empty() || !platform.is_ascii() {
            return Err(TexIndexError::InternalInvariant("invalid release".into()));
        }
        Ok(Self { year, platform })
    }
    #[must_use]
    pub const fn year(&self) -> u16 {
        self.year
    }
    #[must_use]
    pub fn platform(&self) -> &str {
        &self.platform
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum TexFileKind {
    Style,
    Class,
    BibliographyStyle,
    BiblatexStyle,
    BiblatexCitationStyle,
    OpenTypeFont,
    TrueTypeFont,
    Type1Font,
    TeXFontMetric,
    OtherRuntime,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IndexedTexFile {
    relative_path: String,
    basename: String,
    kind: TexFileKind,
    sha256: String,
}
impl IndexedTexFile {
    pub fn new(relative_path: String, sha256: String) -> Result<Self, TexIndexError> {
        validate_hash(&sha256)?;
        let basename = relative_path
            .rsplit('/')
            .next()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| TexIndexError::InternalInvariant("runtime path has no basename".into()))?
            .to_owned();
        let ext = basename
            .rsplit_once('.')
            .map_or("", |(_, e)| e)
            .to_ascii_lowercase();
        let kind = match ext.as_str() {
            "sty" => TexFileKind::Style,
            "cls" => TexFileKind::Class,
            "bst" => TexFileKind::BibliographyStyle,
            "bbx" => TexFileKind::BiblatexStyle,
            "cbx" => TexFileKind::BiblatexCitationStyle,
            "otf" => TexFileKind::OpenTypeFont,
            "ttf" => TexFileKind::TrueTypeFont,
            "pfb" => TexFileKind::Type1Font,
            "tfm" => TexFileKind::TeXFontMetric,
            _ => TexFileKind::OtherRuntime,
        };
        Ok(Self {
            relative_path,
            basename,
            kind,
            sha256,
        })
    }
    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }
    #[must_use]
    pub fn basename(&self) -> &str {
        &self.basename
    }
    #[must_use]
    pub const fn kind(&self) -> TexFileKind {
        self.kind
    }
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TexLivePackage {
    name: String,
    category: String,
    revision: u64,
    catalogue_version: Option<String>,
    catalogue_license: Option<String>,
    runtime_files: Vec<IndexedTexFile>,
}
impl TexLivePackage {
    pub fn new(
        name: String,
        category: String,
        revision: u64,
        catalogue_version: Option<String>,
        catalogue_license: Option<String>,
        mut runtime_files: Vec<IndexedTexFile>,
    ) -> Result<Self, TexIndexError> {
        if name.is_empty() || category.is_empty() {
            return Err(TexIndexError::InternalInvariant("invalid package".into()));
        }
        runtime_files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(Self {
            name,
            category,
            revision,
            catalogue_version,
            catalogue_license,
            runtime_files,
        })
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub fn catalogue_version(&self) -> Option<&str> {
        self.catalogue_version.as_deref()
    }
    #[must_use]
    pub fn catalogue_license(&self) -> Option<&str> {
        self.catalogue_license.as_deref()
    }
    #[must_use]
    pub fn runtime_files(&self) -> &[IndexedTexFile] {
        &self.runtime_files
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TexConfigRecord {
    logical_name: String,
    sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct IdentityPackageV1 {
    name: String,
    revision: u64,
    files: Vec<(String, String)>,
}

/// Canonical output-affecting material used to derive a TeX environment ID.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TexEnvironmentIdentityV1 {
    schema_version: u32,
    release_year: u16,
    platform: String,
    packages: Vec<IdentityPackageV1>,
    tools: BTreeMap<TexToolKind, TexToolRecord>,
    config_files: BTreeMap<String, TexConfigRecord>,
}
impl TexEnvironmentIdentityV1 {
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, TexIndexError> {
        serde_json::to_vec(self).map_err(|error| TexIndexError::Serialization(error.to_string()))
    }
}
impl TexConfigRecord {
    pub fn new(logical_name: String, sha256: String) -> Result<Self, TexIndexError> {
        validate_hash(&sha256)?;
        if logical_name.is_empty() {
            return Err(TexIndexError::InternalInvariant("empty config name".into()));
        }
        Ok(Self {
            logical_name,
            sha256,
        })
    }
    #[must_use]
    pub fn logical_name(&self) -> &str {
        &self.logical_name
    }
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// Immutable snapshot of one installed TeX environment with a content-derived
/// identity suitable for future compilation cache keys.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TexEnvironmentIndexV1 {
    schema_version: u32,
    release: TexLiveRelease,
    packages: BTreeMap<String, TexLivePackage>,
    tools: BTreeMap<TexToolKind, TexToolRecord>,
    config_files: BTreeMap<String, TexConfigRecord>,
    #[serde(skip)]
    files_by_basename: BTreeMap<String, Vec<(String, IndexedTexFile)>>,
}
impl<'de> Deserialize<'de> for TexEnvironmentIndexV1 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: u32,
            release: TexLiveRelease,
            packages: BTreeMap<String, TexLivePackage>,
            tools: BTreeMap<TexToolKind, TexToolRecord>,
            config_files: BTreeMap<String, TexConfigRecord>,
        }
        let w = Wire::deserialize(d)?;
        if w.schema_version != 1 {
            return Err(de::Error::custom(TexIndexError::UnsupportedSchema(
                w.schema_version,
            )));
        }
        Self::new(w.release, w.packages, w.tools, w.config_files).map_err(de::Error::custom)
    }
}
impl TexEnvironmentIndexV1 {
    pub const SCHEMA_VERSION: u32 = 1;
    /// Creates an immutable snapshot of one installed environment.
    pub fn new(
        release: TexLiveRelease,
        packages: BTreeMap<String, TexLivePackage>,
        tools: BTreeMap<TexToolKind, TexToolRecord>,
        config_files: BTreeMap<String, TexConfigRecord>,
    ) -> Result<Self, TexIndexError> {
        for (n, p) in &packages {
            if n != p.name() {
                return Err(TexIndexError::InternalInvariant(
                    "package map key mismatch".into(),
                ));
            }
            for f in p.runtime_files() {
                validate_hash(f.sha256())?;
            }
        }
        for (k, t) in &tools {
            if *k != t.kind() {
                return Err(TexIndexError::InternalInvariant(
                    "tool map key mismatch".into(),
                ));
            }
            validate_hash(t.binary_sha256())?;
        }
        for (n, c) in &config_files {
            if n != c.logical_name() {
                return Err(TexIndexError::InternalInvariant(
                    "config map key mismatch".into(),
                ));
            }
            validate_hash(c.sha256())?;
        }
        let mut value = Self {
            schema_version: 1,
            release,
            packages,
            tools,
            config_files,
            files_by_basename: BTreeMap::new(),
        };
        value.rebuild_queries();
        Ok(value)
    }
    fn rebuild_queries(&mut self) {
        for (n, p) in &self.packages {
            for f in p.runtime_files() {
                self.files_by_basename
                    .entry(f.basename.clone())
                    .or_default()
                    .push((n.clone(), f.clone()));
            }
        }
    }
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }
    #[must_use]
    pub const fn release(&self) -> &TexLiveRelease {
        &self.release
    }
    #[must_use]
    pub fn packages(&self) -> &BTreeMap<String, TexLivePackage> {
        &self.packages
    }
    #[must_use]
    pub fn tools(&self) -> &BTreeMap<TexToolKind, TexToolRecord> {
        &self.tools
    }
    #[must_use]
    pub fn config_files(&self) -> &BTreeMap<String, TexConfigRecord> {
        &self.config_files
    }
    #[must_use]
    pub fn package_exists(&self, n: &str) -> bool {
        self.packages.contains_key(n)
    }
    #[must_use]
    pub fn package(&self, n: &str) -> Option<&TexLivePackage> {
        self.packages.get(n)
    }
    #[must_use]
    pub fn tool(&self, k: TexToolKind) -> Option<&TexToolRecord> {
        self.tools.get(&k)
    }
    fn exists_kind(&self, n: &str, suffix: &str, kinds: &[TexFileKind]) -> bool {
        let key = if n.ends_with(suffix) {
            n.to_owned()
        } else {
            format!("{n}{suffix}")
        };
        self.files_by_basename
            .get(&key)
            .is_some_and(|v| v.iter().any(|(_, f)| kinds.contains(&f.kind)))
    }
    #[must_use]
    pub fn style_exists(&self, n: &str) -> bool {
        self.exists_kind(n, ".sty", &[TexFileKind::Style])
    }
    #[must_use]
    pub fn class_exists(&self, n: &str) -> bool {
        self.exists_kind(n, ".cls", &[TexFileKind::Class])
    }
    #[must_use]
    pub fn bibliography_style_exists(&self, n: &str) -> bool {
        self.exists_kind(n, ".bst", &[TexFileKind::BibliographyStyle])
    }
    #[must_use]
    pub fn biblatex_style_exists(&self, n: &str) -> bool {
        self.exists_kind(n, ".bbx", &[TexFileKind::BiblatexStyle])
            || self.exists_kind(n, ".cbx", &[TexFileKind::BiblatexCitationStyle])
    }
    #[must_use]
    pub fn font_exists(&self, n: &str) -> bool {
        self.files_by_basename.get(n).is_some_and(|v| {
            v.iter().any(|(_, f)| {
                matches!(
                    f.kind,
                    TexFileKind::OpenTypeFont
                        | TexFileKind::TrueTypeFont
                        | TexFileKind::Type1Font
                        | TexFileKind::TeXFontMetric
                )
            })
        })
    }
    #[must_use]
    pub fn find_files_by_basename(&self, n: &str) -> Vec<&IndexedTexFile> {
        self.files_by_basename
            .get(n)
            .map_or_else(Vec::new, |v| v.iter().map(|(_, f)| f).collect())
    }
    #[must_use]
    pub fn owners_of_basename(&self, n: &str) -> Vec<&str> {
        self.files_by_basename
            .get(n)
            .map_or_else(Vec::new, |v| v.iter().map(|(o, _)| o.as_str()).collect())
    }
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, TexIndexError> {
        serde_json::to_vec(self).map_err(|e| TexIndexError::Serialization(e.to_string()))
    }
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, TexIndexError> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| TexIndexError::Serialization(error.to_string()))?;
        let schema = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| TexIndexError::Serialization("missing schema_version".into()))?;
        if schema != u64::from(Self::SCHEMA_VERSION) {
            return Err(TexIndexError::UnsupportedSchema(
                u32::try_from(schema).unwrap_or(u32::MAX),
            ));
        }
        serde_json::from_slice(bytes).map_err(|e| TexIndexError::Serialization(e.to_string()))
    }
    pub fn environment_id(&self) -> Result<TexEnvironmentId, TexIndexError> {
        let material = self.identity_material();
        let bytes = material.canonical_json_bytes()?;
        let hash = hex::encode(Sha256::digest(bytes));
        TexEnvironmentId::parse(&format!("texlive-{}-sha256-{hash}", self.release.year))
            .map_err(|e| TexIndexError::InternalInvariant(e.to_string()))
    }
    #[must_use]
    pub fn identity_material(&self) -> TexEnvironmentIdentityV1 {
        let packages = self
            .packages
            .values()
            .map(|p| IdentityPackageV1 {
                name: p.name().to_owned(),
                revision: p.revision(),
                files: p
                    .runtime_files()
                    .iter()
                    .map(|f| (f.relative_path().to_owned(), f.sha256().to_owned()))
                    .collect(),
            })
            .collect();
        TexEnvironmentIdentityV1 {
            schema_version: 1,
            release_year: self.release.year,
            platform: self.release.platform.clone(),
            packages,
            tools: self.tools.clone(),
            config_files: self.config_files.clone(),
        }
    }
    #[must_use]
    pub fn runtime_file_count(&self) -> usize {
        self.packages.values().map(|p| p.runtime_files.len()).sum()
    }
}
fn validate_hash(value: &str) -> Result<(), TexIndexError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(TexIndexError::InternalInvariant(format!(
            "invalid SHA-256: {value}"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TexResolvedFile {
    relative_or_logical_name: String,
    physical_path: PathBuf,
}
impl TexResolvedFile {
    #[must_use]
    pub fn new(relative_or_logical_name: String, physical_path: PathBuf) -> Self {
        Self {
            relative_or_logical_name,
            physical_path,
        }
    }
    #[must_use]
    pub fn relative_or_logical_name(&self) -> &str {
        &self.relative_or_logical_name
    }
    #[must_use]
    pub fn physical_path(&self) -> &std::path::Path {
        &self.physical_path
    }
}
