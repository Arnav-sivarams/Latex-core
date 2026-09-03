//! Front Matter Pack manifest validation and non-executable placeholder rendering.
#![forbid(unsafe_code)]

use crate::archive::{ImportedArchive, ImportedFile};
use bytes::Bytes;
use core_types::LogicalPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const MANIFEST_PATH: &str = "frontmatter.json";
pub const MANAGED_ROOT: &str = ".latex-core/frontmatter";
pub const INTEGRATION_MARKER: &str =
    "\\input{.latex-core/frontmatter/frontmatter.tex} % LATEX_CORE_FRONT_MATTER";

const ALLOWED_SOURCES: &[&str] = &[
    "team.name",
    "team.academic_year",
    "team.semester",
    "team.dominant_programme_code",
    "writers.names",
    "writers.registration_numbers",
    "writers.names_and_registration_numbers",
    "leader.name",
    "leader.registration_number",
    "mentor.name",
    "mentor.honorific",
    "mentor.designation",
    "mentor.faculty_id",
    "department.id",
    "school.id",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontMatterManifest {
    pub schema_version: u32,
    pub entry_file: String,
    pub sections: Vec<FrontMatterSection>,
    pub fields: Vec<FrontMatterField>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontMatterSection {
    pub key: String,
    pub label: String,
    pub file: String,
    pub required: bool,
    pub default_enabled: bool,
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FrontMatterFieldType {
    Text,
    Multiline,
    Date,
    Boolean,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontMatterField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: FrontMatterFieldType,
    pub required: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub allow_team_override: bool,
}

#[derive(Clone, Debug)]
pub struct ValidatedPack {
    pub manifest: FrontMatterManifest,
    pub files: Vec<ImportedFile>,
}

#[derive(Clone, Debug)]
pub struct ResolvedValue {
    pub value: Value,
    pub source: &'static str,
}

#[derive(Clone, Debug)]
pub struct RenderedFile {
    pub path: LogicalPath,
    pub bytes: Bytes,
}

#[derive(Clone, Debug)]
pub struct RenderedPack {
    pub files: Vec<RenderedFile>,
    pub resolved: BTreeMap<String, ResolvedValue>,
    pub enabled_sections: BTreeMap<String, bool>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum FrontMatterError {
    #[error("frontmatter.json is required")]
    MissingManifest,
    #[error("frontmatter.json must be valid UTF-8 JSON")]
    InvalidManifest,
    #[error("unsupported Front Matter schema version")]
    SchemaVersion,
    #[error("Front Matter Pack must contain at least one .tex file")]
    MissingTex,
    #[error("Front Matter Pack contains an unsupported file: {0}")]
    UnsupportedFile(String),
    #[error("manifest path is invalid: {0}")]
    InvalidPath(String),
    #[error("manifest references a missing file: {0}")]
    MissingFile(String),
    #[error("manifest contains a duplicate key: {0}")]
    DuplicateKey(String),
    #[error("manifest key is invalid: {0}")]
    InvalidKey(String),
    #[error("manifest label is invalid")]
    InvalidLabel,
    #[error("automatic field source is not allowed: {0}")]
    UnsupportedSource(String),
    #[error("default value has the wrong type for field: {0}")]
    InvalidDefault(String),
    #[error("unknown placeholder: {0}")]
    UnknownPlaceholder(String),
    #[error("malformed placeholder in: {0}")]
    MalformedPlaceholder(String),
    #[error("required Front Matter fields need information")]
    MissingRequired(Vec<String>),
    #[error("invalid value for field: {0}")]
    InvalidValue(String),
}

pub fn validate_archive(archive: ImportedArchive) -> Result<ValidatedPack, FrontMatterError> {
    let manifest_file = archive
        .files
        .iter()
        .find(|file| file.path.as_str() == MANIFEST_PATH)
        .ok_or(FrontMatterError::MissingManifest)?;
    if !archive
        .files
        .iter()
        .any(|file| file.path.extension() == Some("tex"))
    {
        return Err(FrontMatterError::MissingTex);
    }
    for file in &archive.files {
        let path = file.path.as_str();
        let extension = file.path.extension().unwrap_or("").to_ascii_lowercase();
        let allowed = matches!(
            extension.as_str(),
            "tex" | "png" | "jpg" | "jpeg" | "pdf" | "bib"
        ) || path == MANIFEST_PATH;
        if !allowed {
            return Err(FrontMatterError::UnsupportedFile(path.to_owned()));
        }
    }
    let manifest: FrontMatterManifest = serde_json::from_slice(&manifest_file.bytes)
        .map_err(|_| FrontMatterError::InvalidManifest)?;
    validate_manifest(&manifest, &archive.files)?;
    Ok(ValidatedPack {
        manifest,
        files: archive.files,
    })
}

pub fn validate_manifest(
    manifest: &FrontMatterManifest,
    files: &[ImportedFile],
) -> Result<(), FrontMatterError> {
    if manifest.schema_version != 1 {
        return Err(FrontMatterError::SchemaVersion);
    }
    let paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    validate_referenced_tex(&manifest.entry_file, &paths)?;
    let mut section_keys = BTreeSet::new();
    for section in &manifest.sections {
        validate_key(&section.key)?;
        validate_label(&section.label)?;
        if !section_keys.insert(section.key.as_str()) {
            return Err(FrontMatterError::DuplicateKey(section.key.clone()));
        }
        if section.required && !section.default_enabled {
            return Err(FrontMatterError::InvalidDefault(section.key.clone()));
        }
        validate_referenced_tex(&section.file, &paths)?;
    }
    let mut field_keys = BTreeSet::new();
    for field in &manifest.fields {
        validate_key(&field.key)?;
        validate_label(&field.label)?;
        if !field_keys.insert(field.key.as_str()) {
            return Err(FrontMatterError::DuplicateKey(field.key.clone()));
        }
        if let Some(source) = field.source.as_deref()
            && !ALLOWED_SOURCES.contains(&source)
        {
            return Err(FrontMatterError::UnsupportedSource(source.to_owned()));
        }
        if let Some(default) = &field.default
            && !value_matches(field.field_type, default)
        {
            return Err(FrontMatterError::InvalidDefault(field.key.clone()));
        }
    }
    for file in files
        .iter()
        .filter(|file| file.path.extension() == Some("tex"))
    {
        let text =
            std::str::from_utf8(&file.bytes).map_err(|_| FrontMatterError::InvalidManifest)?;
        for placeholder in placeholders(text, file.path.as_str())? {
            if !field_keys.contains(placeholder.as_str()) {
                return Err(FrontMatterError::UnknownPlaceholder(placeholder));
            }
        }
    }
    Ok(())
}

pub fn main_template_compatible(main: &[u8]) -> bool {
    std::str::from_utf8(main)
        .is_ok_and(|value| value.lines().any(|line| line.trim() == INTEGRATION_MARKER))
}

#[allow(
    clippy::too_many_lines,
    reason = "validation, precedence resolution, section selection, and deterministic rendering form one atomic transformation"
)]
pub fn resolve_and_render(
    pack: &ValidatedPack,
    automatic: &BTreeMap<String, Value>,
    overrides: &BTreeMap<String, Value>,
    section_choices: &BTreeMap<String, bool>,
) -> Result<RenderedPack, FrontMatterError> {
    for key in overrides.keys() {
        let field = pack
            .manifest
            .fields
            .iter()
            .find(|field| &field.key == key)
            .ok_or_else(|| FrontMatterError::InvalidValue(key.clone()))?;
        if !field.allow_team_override {
            return Err(FrontMatterError::InvalidValue(key.clone()));
        }
    }
    for key in section_choices.keys() {
        if !pack
            .manifest
            .sections
            .iter()
            .any(|section| &section.key == key)
        {
            return Err(FrontMatterError::InvalidValue(key.clone()));
        }
    }
    let mut resolved = BTreeMap::new();
    let mut missing = Vec::new();
    for field in &pack.manifest.fields {
        let selected = if field.allow_team_override {
            overrides
                .get(&field.key)
                .map(|value| (value, "TEAM_OVERRIDE"))
        } else {
            None
        }
        .or_else(|| {
            field
                .source
                .as_ref()
                .and_then(|source| automatic.get(source))
                .map(|value| (value, "AUTO"))
        })
        .or_else(|| field.default.as_ref().map(|value| (value, "PACK_DEFAULT")));
        if let Some((value, source)) = selected {
            if !value_matches(field.field_type, value) {
                return Err(FrontMatterError::InvalidValue(field.key.clone()));
            }
            if field.required && empty_value(value) {
                missing.push(field.label.clone());
            }
            resolved.insert(
                field.key.clone(),
                ResolvedValue {
                    value: value.clone(),
                    source,
                },
            );
        } else if field.required {
            missing.push(field.label.clone());
        }
    }
    if !missing.is_empty() {
        return Err(FrontMatterError::MissingRequired(missing));
    }
    let enabled_sections = pack
        .manifest
        .sections
        .iter()
        .map(|section| {
            let enabled = section.required
                || section_choices
                    .get(&section.key)
                    .copied()
                    .unwrap_or(section.default_enabled);
            (section.key.clone(), enabled)
        })
        .collect::<BTreeMap<_, _>>();
    let disabled_files = pack
        .manifest
        .sections
        .iter()
        .filter(|section| !enabled_sections[&section.key])
        .map(|section| section.file.as_str())
        .collect::<BTreeSet<_>>();
    let mut rendered = Vec::new();
    for file in &pack.files {
        if file.path.as_str() == MANIFEST_PATH {
            continue;
        }
        let relative = file.path.as_str();
        let path = LogicalPath::parse(&format!("{MANAGED_ROOT}/{relative}"))
            .map_err(|_| FrontMatterError::InvalidPath(relative.to_owned()))?;
        let bytes = if file.path.extension() == Some("tex") {
            if disabled_files.contains(file.path.as_str()) {
                Bytes::from_static(b"% Disabled by document details.\n")
            } else {
                let source = std::str::from_utf8(&file.bytes)
                    .map_err(|_| FrontMatterError::InvalidManifest)?;
                Bytes::from(render_tex(
                    source,
                    file.path.as_str(),
                    &pack.manifest,
                    &resolved,
                )?)
            }
        } else {
            file.bytes.clone()
        };
        rendered.push(RenderedFile { path, bytes });
    }
    if pack.manifest.entry_file != "frontmatter.tex" {
        rendered.push(RenderedFile {
            path: LogicalPath::parse(&format!("{MANAGED_ROOT}/frontmatter.tex"))
                .expect("static managed path is valid"),
            bytes: Bytes::from(format!("\\input{{{}}}\n", pack.manifest.entry_file)),
        });
    }
    rendered.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(RenderedPack {
        files: rendered,
        resolved,
        enabled_sections,
    })
}

fn render_tex(
    source: &str,
    path: &str,
    manifest: &FrontMatterManifest,
    values: &BTreeMap<String, ResolvedValue>,
) -> Result<String, FrontMatterError> {
    let fields = manifest
        .fields
        .iter()
        .map(|field| (field.key.as_str(), field))
        .collect::<BTreeMap<_, _>>();
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("{{") {
        let start = cursor + relative;
        output.push_str(&source[cursor..start]);
        let value_start = start + 2;
        let Some(relative_end) = source[value_start..].find("}}") else {
            return Err(FrontMatterError::MalformedPlaceholder(path.to_owned()));
        };
        let end = value_start + relative_end;
        let key = source[value_start..end].trim();
        let field = fields
            .get(key)
            .ok_or_else(|| FrontMatterError::UnknownPlaceholder(key.to_owned()))?;
        if let Some(value) = values.get(key) {
            output.push_str(&render_value(field.field_type, &value.value));
        }
        cursor = end + 2;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

fn placeholders(source: &str, path: &str) -> Result<Vec<String>, FrontMatterError> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("{{") {
        let start = cursor + relative + 2;
        let Some(relative_end) = source[start..].find("}}") else {
            return Err(FrontMatterError::MalformedPlaceholder(path.to_owned()));
        };
        let end = start + relative_end;
        let key = source[start..end].trim();
        validate_key(key)?;
        found.push(key.to_owned());
        cursor = end + 2;
    }
    Ok(found)
}

fn validate_referenced_tex(value: &str, paths: &BTreeSet<&str>) -> Result<(), FrontMatterError> {
    let path =
        LogicalPath::parse(value).map_err(|_| FrontMatterError::InvalidPath(value.to_owned()))?;
    if path.extension() != Some("tex") {
        return Err(FrontMatterError::InvalidPath(value.to_owned()));
    }
    if !paths.contains(path.as_str()) {
        return Err(FrontMatterError::MissingFile(value.to_owned()));
    }
    Ok(())
}

fn validate_key(value: &str) -> Result<(), FrontMatterError> {
    let mut chars = value.chars();
    let valid_first = chars.next().is_some_and(|ch| ch.is_ascii_lowercase());
    if !valid_first
        || value.len() > 100
        || !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(FrontMatterError::InvalidKey(value.to_owned()));
    }
    Ok(())
}

fn validate_label(value: &str) -> Result<(), FrontMatterError> {
    if value.trim().is_empty() || value.chars().count() > 200 {
        Err(FrontMatterError::InvalidLabel)
    } else {
        Ok(())
    }
}

fn value_matches(field_type: FrontMatterFieldType, value: &Value) -> bool {
    match field_type {
        FrontMatterFieldType::Boolean => value.is_boolean(),
        FrontMatterFieldType::Date => value.as_str().is_some_and(valid_date),
        FrontMatterFieldType::Text => value.is_string(),
        FrontMatterFieldType::Multiline => {
            value.is_string()
                || value
                    .as_array()
                    .is_some_and(|items| items.iter().all(Value::is_string))
        }
    }
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit()))
    {
        return false;
    }
    let year = value[0..4].parse::<u32>().unwrap_or_default();
    let month = value[5..7].parse::<u32>().unwrap_or_default();
    let day = value[8..10].parse::<u32>().unwrap_or_default();
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let maximum = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=maximum).contains(&day)
}

fn empty_value(value: &Value) -> bool {
    value.as_str().is_some_and(|text| text.trim().is_empty())
        || value.as_array().is_some_and(Vec::is_empty)
}

fn render_value(field_type: FrontMatterFieldType, value: &Value) -> String {
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter_map(Value::as_str)
            .map(escape_latex_text)
            .collect::<Vec<_>>()
            .join("\\\\\n");
    }
    if let Some(value) = value.as_bool() {
        return if value { "true" } else { "false" }.to_owned();
    }
    let text = value.as_str().unwrap_or_default();
    if field_type == FrontMatterFieldType::Multiline {
        text.lines()
            .map(escape_latex_text)
            .collect::<Vec<_>>()
            .join("\\par\n")
    } else {
        escape_latex_text(&text.replace(['\r', '\n'], " "))
    }
}

pub fn escape_latex_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\textbackslash{}"),
            '{' => escaped.push_str("\\{"),
            '}' => escaped.push_str("\\}"),
            '$' => escaped.push_str("\\$"),
            '&' => escaped.push_str("\\&"),
            '#' => escaped.push_str("\\#"),
            '%' => escaped.push_str("\\%"),
            '_' => escaped.push_str("\\_"),
            '^' => escaped.push_str("\\textasciicircum{}"),
            '~' => escaped.push_str("\\textasciitilde{}"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "small deterministic in-memory fixtures should fail immediately at their construction site"
)]
mod tests {
    use super::*;

    fn fixture(manifest: &str, tex: &[(&str, &str)]) -> ImportedArchive {
        let mut files = vec![ImportedFile {
            path: LogicalPath::parse(MANIFEST_PATH).unwrap(),
            bytes: Bytes::copy_from_slice(manifest.as_bytes()),
        }];
        files.extend(tex.iter().map(|(path, value)| ImportedFile {
            path: LogicalPath::parse(path).unwrap(),
            bytes: Bytes::copy_from_slice(value.as_bytes()),
        }));
        ImportedArchive {
            files,
            detected_main: None,
        }
    }

    fn manifest(source: &str) -> String {
        format!(
            r#"{{"schema_version":1,"entry_file":"frontmatter.tex","sections":[{{"key":"cover","label":"Cover","file":"cover.tex","required":true,"default_enabled":true}}],"fields":[{{"key":"title","label":"Title","type":"TEXT","required":true,"source":"{source}","default":null,"allow_team_override":false}}]}}"#
        )
    }

    #[test]
    fn validates_allow_list_paths_and_placeholders() {
        let value = manifest("team.name");
        let pack = validate_archive(fixture(
            &value,
            &[
                ("frontmatter.tex", "\\input{cover.tex}"),
                ("cover.tex", "{{title}}"),
            ],
        ))
        .unwrap();
        assert_eq!(pack.manifest.fields.len(), 1);
        for bad_source in ["department.name", "sql:select name", "institution.name"] {
            let value = manifest(bad_source);
            assert!(matches!(
                validate_archive(fixture(
                    &value,
                    &[("frontmatter.tex", "x"), ("cover.tex", "{{title}}")]
                )),
                Err(FrontMatterError::UnsupportedSource(_))
            ));
        }
    }

    #[test]
    fn rejects_unknown_placeholder_and_missing_files() {
        let value = manifest("team.name");
        assert!(matches!(
            validate_archive(fixture(
                &value,
                &[("frontmatter.tex", "{{other}}"), ("cover.tex", "x")]
            )),
            Err(FrontMatterError::UnknownPlaceholder(_))
        ));
        assert!(matches!(
            validate_archive(fixture(&value, &[("frontmatter.tex", "{{title}}")])),
            Err(FrontMatterError::MissingFile(_))
        ));
    }

    #[test]
    fn exact_main_template_marker_is_required() {
        assert!(main_template_compatible(
            format!("\\begin{{document}}\n{INTEGRATION_MARKER}\n").as_bytes()
        ));
        assert!(!main_template_compatible(
            b"\\begin{document}\n\\input{frontmatter.tex}"
        ));
        assert!(!main_template_compatible(
            b"\\input{.latex-core/frontmatter/frontmatter.tex}"
        ));
    }

    #[test]
    fn values_are_latex_text_and_never_control_sequences() {
        let value = manifest("team.name");
        let pack = validate_archive(fixture(
            &value,
            &[("frontmatter.tex", "{{title}}"), ("cover.tex", "{{title}}")],
        ))
        .unwrap();
        let attack = r"\input{/etc/passwd} \write18 <script> & % _";
        let rendered = resolve_and_render(
            &pack,
            &BTreeMap::from([("team.name".to_owned(), Value::String(attack.to_owned()))]),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        let entry = std::str::from_utf8(
            &rendered
                .files
                .iter()
                .find(|file| file.path.as_str().ends_with("frontmatter.tex"))
                .unwrap()
                .bytes,
        )
        .unwrap();
        assert!(!entry.contains(r"\input{/etc/passwd}"));
        assert!(!entry.contains(r"\write18"));
        assert!(entry.contains(r"\textbackslash{}input\{/etc/passwd\}"));
        assert!(entry.contains(r"\& \% \_"));
        assert!(entry.contains("<script>"));
    }

    #[test]
    fn required_sections_cannot_be_disabled_and_optional_files_can() {
        let value = r#"{"schema_version":1,"entry_file":"frontmatter.tex","sections":[{"key":"cover","label":"Cover","file":"cover.tex","required":true,"default_enabled":true},{"key":"thanks","label":"Thanks","file":"thanks.tex","required":false,"default_enabled":true}],"fields":[]}"#;
        let pack = validate_archive(fixture(
            value,
            &[
                ("frontmatter.tex", "x"),
                ("cover.tex", "cover"),
                ("thanks.tex", "thanks"),
            ],
        ))
        .unwrap();
        let rendered = resolve_and_render(
            &pack,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::from([("cover".into(), false), ("thanks".into(), false)]),
        )
        .unwrap();
        assert!(rendered.enabled_sections["cover"]);
        assert!(!rendered.enabled_sections["thanks"]);
        assert_eq!(
            rendered
                .files
                .iter()
                .find(|file| file.path.as_str().ends_with("thanks.tex"))
                .unwrap()
                .bytes,
            Bytes::from_static(b"% Disabled by document details.\n")
        );
    }

    #[test]
    fn dates_and_text_shapes_are_strict() {
        assert!(valid_date("2028-02-29"));
        assert!(!valid_date("2026-02-29"));
        assert!(!valid_date("2026-13-01"));
        assert!(!value_matches(
            FrontMatterFieldType::Text,
            &serde_json::json!(["not", "text"])
        ));
        assert!(value_matches(
            FrontMatterFieldType::Multiline,
            &serde_json::json!(["Writer A", "Writer B"])
        ));
    }
}
