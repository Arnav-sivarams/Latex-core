use crate::TexIndexError;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlpdbPackageRecord {
    name: String,
    category: String,
    revision: u64,
    catalogue_version: Option<String>,
    catalogue_license: Option<String>,
    runfiles: Vec<String>,
    binfiles: Vec<String>,
}
impl TlpdbPackageRecord {
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
    pub fn runfiles(&self) -> &[String] {
        &self.runfiles
    }
    #[must_use]
    pub fn binfiles(&self) -> &[String] {
        &self.binfiles
    }
}

pub fn parse_tlpdb(input: &str) -> Result<BTreeMap<String, TlpdbPackageRecord>, TexIndexError> {
    let mut result = BTreeMap::new();
    let mut seen_names = BTreeSet::new();
    for block in input.replace("\r\n", "\n").split("\n\n") {
        if block.trim().is_empty() {
            continue;
        }
        let mut name = None;
        let mut category = None;
        let mut revision = None;
        let mut catalogue_version = None;
        let mut catalogue_license = None;
        let mut runfiles = Vec::new();
        let mut binfiles = Vec::new();
        let mut mode = 0u8;
        for line in block.lines() {
            if let Some(path) = line.strip_prefix(' ') {
                if mode == 1 {
                    validate_relative_package_path(path)?;
                    runfiles.push(path.to_owned());
                } else if mode == 2 {
                    validate_relative_package_path(path)?;
                    binfiles.push(path.to_owned());
                }
                continue;
            }
            mode = 0;
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "name" => name = Some(value.to_owned()),
                "category" => category = Some(value.to_owned()),
                "revision" => {
                    revision = Some(value.parse::<u64>().map_err(|_| {
                        TexIndexError::InvalidTlpdb(format!("invalid revision: {value}"))
                    })?);
                }
                "catalogue-version" => catalogue_version = Some(value.to_owned()),
                "catalogue-license" => catalogue_license = Some(value.to_owned()),
                "runfiles" => mode = 1,
                "binfiles" => mode = 2,
                _ => {}
            }
        }
        let name = name
            .filter(|v| !v.is_empty())
            .ok_or_else(|| TexIndexError::InvalidTlpdb("record missing name".into()))?;
        let category = category
            .filter(|v| !v.is_empty())
            .ok_or_else(|| TexIndexError::InvalidTlpdb(format!("{name}: missing category")))?;
        if !seen_names.insert(name.clone()) {
            return Err(TexIndexError::InvalidTlpdb(format!(
                "duplicate package: {name}"
            )));
        }
        if matches!(name.as_str(), "00texlive.config" | "00texlive.installation")
            && category == "TLCore"
            && revision.is_none()
        {
            continue;
        }
        let record = TlpdbPackageRecord {
            name: name.clone(),
            category,
            revision: revision
                .ok_or_else(|| TexIndexError::InvalidTlpdb(format!("{name}: missing revision")))?,
            catalogue_version,
            catalogue_license,
            runfiles,
            binfiles,
        };
        result.insert(name, record);
    }
    Ok(result)
}
pub(crate) fn validate_relative_package_path(value: &str) -> Result<(), TexIndexError> {
    if value.is_empty()
        || value.contains('\0')
        || value.contains('\\')
        || Path::new(value).is_absolute()
        || Path::new(value).components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(TexIndexError::InvalidPackagePath(value.to_owned()));
    }
    Ok(())
}
