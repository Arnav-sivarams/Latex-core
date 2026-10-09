//! Front Matter Pack manifest validation and non-executable placeholder rendering.
#![forbid(unsafe_code)]

use crate::archive::{ImportedArchive, ImportedFile};
use bytes::Bytes;
use core_types::LogicalPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use thiserror::Error;

pub mod legacy;

pub const MANIFEST_PATH: &str = "frontmatter.json";
pub const MANAGED_ROOT: &str = ".latex-core/frontmatter";
const MANAGED_NAMESPACE: &str = ".latex-core";
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
    "project.executive_summary",
    "project.type",
    "project.datasets",
    "project.source_code_snippets",
    "project.department_names",
    "project.school_names",
];

pub const SINGLE_SOURCE_MARKER: &str = "% LATEX_CORE_SINGLE_SOURCE_BINDINGS";

/// Institution-approved names; registrations store course IDs, not names.
pub const COURSE_CATALOG: &[(&str, &str)] = &[
    ("BCSE497J", "Project-I"),
    ("BCSE4973", "Project-I"),
    ("MACSE698", "Internship-I/Dissertation-I"),
];

pub fn canonical_course_name(code: &str) -> Option<&'static str> {
    COURSE_CATALOG
        .iter()
        .find(|(candidate, _)| code.trim().eq_ignore_ascii_case(candidate))
        .map(|(_, name)| *name)
}

/// No template or document override participates in registration resolution.
pub fn registration_document_values(
    registrations: &[(String, String, String)],
) -> (BTreeMap<String, Value>, Vec<String>) {
    let mut values = BTreeMap::from([
        ("course_code".into(), Value::Null),
        ("team.academic_year".into(), Value::Null),
        ("team.semester".into(), Value::Null),
    ]);
    let [(code, year, semester)] = registrations else {
        return (
            values,
            vec![if registrations.is_empty() {
                "Assigned Team Leader has no applicable institutional course registration. Ask an Admin to link the Leader's student identity and import/correct the registration for this Team.".into()
            } else {
                "Assigned Team Leader has multiple applicable institutional course registrations. Ask an Admin to correct the Team/course assignment; no registration was chosen.".into()
            }],
        );
    };
    let display = legacy::institutional_semester(semester).unwrap_or(semester);
    // Keep the actual institutional identifier. Normalization is lookup-only.
    values.insert("course_code".into(), Value::String(code.clone()));
    values.insert("team.academic_year".into(), Value::String(year.clone()));
    values.insert("team.semester".into(), Value::String(display.into()));
    let warnings = if code.trim().is_empty() || legacy::document_calendar(display, year).is_err() {
        vec!["Assigned Leader registration has an invalid course, academic year or unsupported semester. Ask an Admin to correct the institutional record.".into()]
    } else {
        Vec::new()
    };
    (values, warnings)
}

/// An intentional name belongs to its saved course, never a different course.
pub fn document_course_name(
    code: &str,
    source: &BTreeMap<String, Value>,
    template: &BTreeMap<String, Value>,
    saved: &BTreeMap<String, Value>,
) -> Option<String> {
    let same_course = |values: &BTreeMap<String, Value>| {
        values
            .get("course_code")
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim().eq_ignore_ascii_case(code.trim()))
    };
    if same_course(source)
        && (same_course(saved) || source.get("course_name") != template.get("course_name"))
    {
        if let Some(name) = source.get("course_name").and_then(Value::as_str) {
            return Some(name.into());
        }
    }
    if same_course(saved)
        && let Some(name) = saved.get("course_name").and_then(Value::as_str)
    {
        return Some(name.into());
    }
    canonical_course_name(code).map(str::to_owned)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SingleSourceBindingPoint {
    ExplicitMarker,
    AutomaticBeforeDocument,
}

fn tex_code_line(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let mut backslashes = 0;
            let mut previous = index;
            while previous > 0 && bytes[previous - 1] == b'\\' {
                backslashes += 1;
                previous -= 1;
            }
            if backslashes % 2 == 0 {
                return &line[..index];
            }
        }
        index += 1;
    }
    line
}

fn uncommented_document_lines(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .flat_map(|(index, line)| {
            tex_code_line(line)
                .match_indices(r"\begin{document}")
                .map(move |_| index)
        })
        .collect()
}

fn declaration_present(line: &str, command: &str) -> bool {
    let code = tex_code_line(line);
    [r"\newcommand", r"\providecommand", r"\DeclareRobustCommand"]
        .iter()
        .any(|declaration| {
            code.match_indices(declaration).any(|(index, _)| {
                let after = &code[index + declaration.len()..];
                !after
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_alphabetic())
                    && after.trim_start().starts_with(&format!(r"{{\{command}}}"))
            })
        })
}

fn known_declaration_before(lines: &[&str], end: usize) -> bool {
    legacy::REGISTRY.iter().any(|(command, _, _, _)| {
        lines[..end]
            .iter()
            .any(|line| declaration_present(line, command))
    })
}

pub fn single_source_binding_point(main: &[u8]) -> Option<SingleSourceBindingPoint> {
    let source = std::str::from_utf8(main).ok()?;
    let lines = source.lines().collect::<Vec<_>>();
    let documents = uncommented_document_lines(source);
    let [document_line] = documents.as_slice() else {
        return None;
    };
    let markers = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.trim() == SINGLE_SOURCE_MARKER).then_some(index))
        .collect::<Vec<_>>();
    if let [marker] = markers.as_slice()
        && *marker < *document_line
        && known_declaration_before(&lines, *marker)
    {
        return Some(SingleSourceBindingPoint::ExplicitMarker);
    }
    if markers.is_empty() && known_declaration_before(&lines, *document_line) {
        return Some(SingleSourceBindingPoint::AutomaticBeforeDocument);
    }
    None
}

pub fn single_source_compatible(main: &[u8]) -> bool {
    single_source_binding_point(main).is_some()
}

fn source_binding_inputs(
    values: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, FrontMatterError> {
    let mut canonical = values.clone();
    if let (Some(semester), Some(date)) = (
        values
            .get("team.semester")
            .and_then(Value::as_str)
            .filter(|value| value.parse::<u8>().is_ok()),
        values
            .get("submission_date")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty()),
    ) {
        let derived = legacy::derive_semester_metadata(
            semester,
            date,
            values.get("team.academic_year").and_then(Value::as_str),
        )?;
        canonical
            .entry("team.academic_year".into())
            .or_insert(derived.academic_year.into());
    }
    Ok(canonical)
}

pub fn bind_single_source_values(
    main: &[u8],
    values: &BTreeMap<String, Value>,
) -> Result<Bytes, FrontMatterError> {
    let source = std::str::from_utf8(main).map_err(|_| FrontMatterError::InvalidManifest)?;
    let point = single_source_binding_point(main).ok_or_else(|| {
        FrontMatterError::InvalidValue(
            "complete report template metadata compatibility: exactly one safe pre-document binding point and a recognized institutional macro declaration are required".into(),
        )
    })?;
    let mut inputs = single_source_managed_values_from_tex(main);
    inputs.extend(values.clone());
    let canonical = source_binding_inputs(&inputs)?;
    let rendered = legacy::single_source_bindings(&canonical)?;
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let marker = source.find(SINGLE_SOURCE_MARKER);
    let target = match point {
        SingleSourceBindingPoint::ExplicitMarker => {
            marker.ok_or(FrontMatterError::InvalidManifest)?
        }
        SingleSourceBindingPoint::AutomaticBeforeDocument => source
            .split_inclusive('\n')
            .take(uncommented_document_lines(source)[0])
            .map(str::len)
            .sum(),
    };
    let header = "% Known institutional single-source bindings.";
    let start = source[..target].rfind(header).unwrap_or(target);
    let end = marker.map_or(target, |_| target + SINGLE_SOURCE_MARKER.len());
    let commands = legacy::REGISTRY
        .iter()
        .map(|(command, _, _, _)| *command)
        .chain(CANONICAL_INPUTS.iter().map(|(_, command)| *command))
        .collect::<BTreeSet<_>>();
    let preserved = preserve_unmanaged_binding_lines(&source[start..target], header, &commands);
    let mut preamble = source[..start].to_owned();
    let mut binding = format!("{header}\n");
    for command in commands {
        let needle = format!("\\renewcommand{{\\{command}}}");
        let value = rendered
            .find(&needle)
            .and_then(|offset| literal_group(&rendered[offset + needle.len()..]))
            .map(str::to_owned)
            .or_else(|| {
                CANONICAL_INPUTS
                    .iter()
                    .find(|(_, name)| *name == command)
                    .and_then(|(key, _)| canonical.get(*key).and_then(Value::as_str))
                    .map(escape_latex_text)
            });
        // Mask comments without changing byte offsets, so multiline literal
        // definitions can be updated while retaining surrounding formatting.
        let code = preamble
            .split_inclusive('\n')
            .fold(String::new(), |mut masked, line| {
                let code = tex_code_line(line);
                masked.push_str(code);
                masked.extend(std::iter::repeat_n(' ', line.len() - code.len()));
                masked
            });
        let mut declarations = Vec::new();
        for kind in ["newcommand", "providecommand", "DeclareRobustCommand"] {
            let needle = format!("\\{kind}{{\\{command}}}");
            for (offset, _) in code.match_indices(&needle) {
                let rest = &code[offset + needle.len()..];
                let trimmed = rest.trim_start();
                if let Some(body) = literal_group(trimmed) {
                    let from = offset + needle.len() + rest.len() - trimmed.len() + 1;
                    declarations.push((from, from + body.len()));
                }
            }
        }
        declarations.sort_unstable();
        if let Some((from, to)) = declarations.first() {
            if let Some(value) = value {
                preamble.replace_range(*from..*to, &value);
            }
        } else {
            writeln!(
                binding,
                "\\newcommand{{\\{command}}}{{{}}}",
                value.unwrap_or_default()
            )
            .map_err(|_| FrontMatterError::InvalidManifest)?;
        }
    }
    binding = binding.replace('\n', newline);
    binding.push_str(&preserved);
    binding.push_str(SINGLE_SOURCE_MARKER);
    let separator = if marker.is_some() { "" } else { newline };
    Ok(Bytes::from(format!(
        "{preamble}{binding}{separator}{}",
        &source[end..]
    )))
}

fn preserve_unmanaged_binding_lines(
    source: &str,
    header: &str,
    commands: &BTreeSet<&str>,
) -> String {
    // Only remove exact generated zero-argument declarations inside our block.
    // Preserve unknown commands, comments, and user renewcommands outside it.
    let mut preserved = String::new();
    for line in source.split_inclusive('\n') {
        if line.trim() == header {
            continue;
        }
        let code = tex_code_line(line).trim();
        let generated = commands.iter().any(|command| {
            ["providecommand", "renewcommand", "newcommand"]
                .iter()
                .any(|kind| {
                    let needle = format!("\\{kind}{{\\{command}}}");
                    code.strip_prefix(&needle)
                        .and_then(|rest| literal_group(rest.trim_start()))
                        .is_some_and(|body| {
                            code.strip_prefix(&needle).is_some_and(|rest| {
                                rest.trim_start()[body.len() + 2..].trim().is_empty()
                            })
                        })
                })
        });
        if generated {
            if let Some(comment) = line
                .get(tex_code_line(line).len()..)
                .filter(|comment| comment.starts_with('%'))
            {
                preserved.push_str(comment);
            }
        } else {
            preserved.push_str(line);
        }
    }
    preserved
}

/// TeX discards spaces after control words; make student separators explicit.
pub fn format_student_front_matter(source: &[u8]) -> Result<Bytes, FrontMatterError> {
    let source = std::str::from_utf8(source).map_err(|_| FrontMatterError::InvalidManifest)?;
    let mut output = source.to_owned();
    for student in ['A', 'B', 'C', 'D'] {
        let command = format!("\\student{student}name");
        for spaces in ["  ", " "] {
            output = output.replace(&format!("{command}{spaces}("), &format!("{command}\\ ("));
        }
    }
    output = output.replace(r"\hspace{1cm}", r"\setlength{\parindent}{1cm}\indent");
    Ok(Bytes::from(output))
}

pub fn document_owned_source(source: &str) -> bool {
    matches!(source, "course_name" | "project.title" | "submission_date")
}

/// The generated allowlist does not introduce new required template fields.
pub fn single_source_commands(source: &str) -> BTreeSet<String> {
    let Some(start) = source.find("% Known institutional single-source bindings.") else {
        return legacy::commands(source);
    };
    let Some(end) = source[start..].find(SINGLE_SOURCE_MARKER) else {
        return legacy::commands(source);
    };
    legacy::commands(&format!(
        "{}{}",
        &source[..start],
        &source[start + end + SINGLE_SOURCE_MARKER.len()..]
    ))
}

const CANONICAL_INPUTS: &[(&str, &str)] = &[
    ("submission_date", "latexcoresubmissiondate"),
    ("team.semester", "latexcoresemester"),
    ("team.academic_year", "latexcoreacademicyear"),
];

/// Preserve unmanaged macro bodies when updating only submitted document fields.
pub fn single_source_managed_values_from_tex(main: &[u8]) -> BTreeMap<String, Value> {
    let Ok(source) = std::str::from_utf8(main) else {
        return BTreeMap::new();
    };
    let Some(start) = source.find("% Known institutional single-source bindings.") else {
        return BTreeMap::new();
    };
    let Some(end) = source[start..].find(SINGLE_SOURCE_MARKER) else {
        return BTreeMap::new();
    };
    let managed = &source[start..start + end];
    let mut values = single_source_values_from_tex(managed.as_bytes());
    // A generated empty helper is not an explicit date edit. Keeping it as an
    // input would clear the original month/year on the second partial save.
    // An explicitly submitted empty date still clears them through `values`.
    let commands = legacy::commands(managed);
    if values.get("submission_date").and_then(Value::as_str) == Some("")
        && !commands.contains("thesismonth")
        && !commands.contains("thesisyear")
    {
        values.remove("submission_date");
    }
    values
}

/// Read literal macro definitions from the durable source without executing TeX.
/// Later renewcommands take precedence over original template declarations.
pub fn single_source_values_from_tex(main: &[u8]) -> BTreeMap<String, Value> {
    let Ok(source) = std::str::from_utf8(main) else {
        return BTreeMap::new();
    };
    let code = source
        .lines()
        .map(tex_code_line)
        .collect::<Vec<_>>()
        .join("\n");
    let mut values = BTreeMap::new();
    for (command, key) in legacy::REGISTRY
        .iter()
        .map(|(command, key, _, _)| (*command, *key))
        .chain(
            CANONICAL_INPUTS
                .iter()
                .map(|(key, command)| (*command, *key)),
        )
    {
        let mut definitions = Vec::new();
        for declaration in [
            "newcommand",
            "providecommand",
            "renewcommand",
            "DeclareRobustCommand",
        ] {
            let needle = format!("\\{declaration}{{\\{command}}}");
            for (offset, _) in code.match_indices(&needle) {
                let rest = code[offset + needle.len()..].trim_start();
                if let Some(value) = literal_group(rest) {
                    definitions.push((offset, declaration, value));
                }
            }
        }
        definitions.sort_by_key(|(offset, _, _)| *offset);
        let mut current = None;
        for (_, declaration, value) in definitions {
            if declaration == "providecommand" && value.is_empty() {
                continue;
            }
            if declaration != "providecommand" || current.is_none() {
                current = Some(value);
            }
        }
        if let Some(value) = current {
            values.insert(key.to_owned(), Value::String(unescape_latex_text(value)));
        }
    }
    values
}

fn literal_group(text: &str) -> Option<&str> {
    if !text.starts_with('{') {
        return None;
    }
    let mut depth = 0_u32;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[1..index]);
                }
            }
            _ => {}
        }
    }
    None
}

fn unescape_latex_text(text: &str) -> String {
    let escapes = [
        (r"\textbackslash{}", "\\"),
        (r"\textasciitilde{}", "~"),
        (r"\textasciicircum{}", "^"),
        (r"\&", "&"),
        (r"\%", "%"),
        (r"\$", "$"),
        (r"\#", "#"),
        (r"\_", "_"),
        (r"\{", "{"),
        (r"\}", "}"),
    ];
    let mut remaining = text;
    let mut output = String::new();
    while !remaining.is_empty() {
        if let Some((escaped, literal)) = escapes
            .iter()
            .find(|(escaped, _)| remaining.starts_with(escaped))
        {
            output.push_str(literal);
            remaining = &remaining[escaped.len()..];
        } else if let Some(character) = remaining.chars().next() {
            output.push(character);
            remaining = &remaining[character.len_utf8()..];
        }
    }
    output
}

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

pub fn validate_archive(mut archive: ImportedArchive) -> Result<ValidatedPack, FrontMatterError> {
    if !archive
        .files
        .iter()
        .any(|file| file.path.as_str() == MANIFEST_PATH)
    {
        let manifest = legacy::normalize(&archive.files)?;
        archive.files.push(ImportedFile {
            path: LogicalPath::parse(MANIFEST_PATH)
                .map_err(|_| FrontMatterError::InvalidManifest)?,
            bytes: Bytes::from(
                serde_json::to_vec(&manifest).map_err(|_| FrontMatterError::InvalidManifest)?,
            ),
        });
    }
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
        if path == "Front-Matter.tex" {
            return Err(FrontMatterError::UnsupportedFile(path.to_owned()));
        }
        let extension = file.path.extension().unwrap_or("").to_ascii_lowercase();
        let allowed = matches!(
            extension.as_str(),
            "tex" | "cls" | "png" | "jpg" | "jpeg" | "pdf" | "bib"
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
    if files
        .iter()
        .any(|file| is_application_owned_path(file.path.as_str()))
    {
        return Err(FrontMatterError::InvalidPath(
            "reserved generated Front Matter path".into(),
        ));
    }
    if !matches!(manifest.schema_version, 1 | 2) {
        return Err(FrontMatterError::SchemaVersion);
    }
    let paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    if manifest.schema_version == 2 {
        let normalized = legacy::normalize(files)?;
        if serde_json::to_value(&normalized).map_err(|_| FrontMatterError::InvalidManifest)?
            != serde_json::to_value(manifest).map_err(|_| FrontMatterError::InvalidManifest)?
        {
            return Err(FrontMatterError::InvalidManifest);
        }
        return Ok(());
    }
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

pub(crate) fn is_application_owned_path(path: &str) -> bool {
    path == MANAGED_NAMESPACE || path.starts_with(".latex-core/")
}

pub fn main_template_compatible(main: &[u8]) -> bool {
    std::str::from_utf8(main).is_ok_and(|value| {
        value.lines().any(|line| {
            let line = line.trim();
            if line == INTEGRATION_MARKER {
                return true;
            }
            let Some(path) = line
                .strip_prefix("\\input{")
                .and_then(|line| line.strip_suffix("} % LATEX_CORE_FRONT_MATTER"))
            else {
                return false;
            };
            let depth = path.matches("../").count();
            depth > 0
                && depth <= 8
                && path.trim_start_matches("../") == ".latex-core/frontmatter/frontmatter.tex"
        })
    })
}

pub fn rebase_generated_wrapper(files: &mut [RenderedFile], main_path: &LogicalPath) {
    let depth = main_path.as_str().matches('/').count();
    if depth == 0 {
        return;
    }
    let prefix = "../".repeat(depth);
    for file in files
        .iter_mut()
        .filter(|file| file.path.as_str() == format!("{MANAGED_ROOT}/frontmatter.tex"))
    {
        if let Ok(source) = std::str::from_utf8(&file.bytes) {
            file.bytes = Bytes::from(source.replace(
                "\\input{.latex-core/frontmatter/",
                &format!("\\input{{{prefix}.latex-core/frontmatter/"),
            ));
        }
    }
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
        let team_metadata = matches!(
            field.source.as_deref(),
            Some("team.semester" | "team.academic_year")
        );
        if !(field.allow_team_override || team_metadata)
            || !value_matches(field.field_type, &overrides[key])
        {
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
    let applicable;
    let overrides = if pack.manifest.schema_version == 2 {
        applicable = legacy::applicable_overrides(automatic, overrides);
        &applicable
    } else {
        overrides
    };
    let mut resolved = BTreeMap::new();
    let mut missing = Vec::new();
    for field in &pack.manifest.fields {
        let auto = field
            .source
            .as_ref()
            .and_then(|source| automatic.get(source))
            .map(|value| (value, "AUTO"));
        let team_metadata = matches!(
            field.source.as_deref(),
            Some("team.semester" | "team.academic_year")
        );
        let team = (field.allow_team_override || team_metadata)
            .then(|| overrides.get(&field.key))
            .flatten()
            .map(|value| (value, "TEAM_OVERRIDE"));
        let selected = if pack.manifest.schema_version == 2 {
            auto.or(team)
        } else {
            team.or(auto)
        }
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
    if pack.manifest.schema_version == 2 {
        let mut files = legacy::render(pack, &resolved, &enabled_sections)?;
        files.push(RenderedFile {
            path: LogicalPath::parse(&format!("{MANAGED_ROOT}/Front-Matter.tex"))
                .expect("static managed path is valid"),
            bytes: Bytes::from(semantic_definitions(&resolved)),
        });
        files.sort_by(|left, right| left.path.cmp(&right.path));
        return Ok(RenderedPack {
            files,
            resolved,
            enabled_sections,
        });
    }
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
    rendered.push(RenderedFile {
        path: LogicalPath::parse(&format!("{MANAGED_ROOT}/Front-Matter.tex"))
            .expect("static managed path is valid"),
        bytes: Bytes::from(semantic_definitions(&resolved)),
    });
    rendered.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(RenderedPack {
        files: rendered,
        resolved,
        enabled_sections,
    })
}

fn semantic_definitions(values: &BTreeMap<String, ResolvedValue>) -> String {
    let mut output =
        String::from("% Generated by LaTeX Core. Reopen Document details to change values.\n");
    for (key, resolved) in values {
        let command = key
            .split('_')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_ascii_uppercase().to_string() + chars.as_str()
                })
            })
            .collect::<String>();
        let value = if let Some(items) = resolved.value.as_array() {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(escape_latex_text)
                .collect::<Vec<_>>()
                .join("\\\\\n")
        } else {
            escape_latex_text(resolved.value.as_str().unwrap_or_default())
        };
        let _ = writeln!(output, "% canonical: {key}; origin: {}", resolved.source);
        let _ = writeln!(
            output,
            "\\expandafter\\def\\csname LatexCore{command}\\endcsname{{{value}}}"
        );
    }
    output
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
    #[test]
    fn empty_generated_date_helper_preserves_originals_and_explicit_clears() {
        let source = br"\documentclass{article}
\newcommand{\coursecode}{BCSE4973}
\newcommand{\thesismonth}{Month}
\newcommand{\thesisyear}{Year}
\begin{document}Report\end{document}";
        let values = BTreeMap::from([("course_code".into(), Value::String("BCSE4973".into()))]);
        let saved = bind_single_source_values(source, &values).unwrap();
        assert_eq!(bind_single_source_values(&saved, &values).unwrap(), saved);
        let text = String::from_utf8_lossy(&saved);
        assert!(text.contains(r"\newcommand{\thesismonth}{Month}"));
        assert!(text.contains(r"\newcommand{\thesisyear}{Year}"));
        let cleared = bind_single_source_values(
            &saved,
            &BTreeMap::from([("submission_date".into(), Value::String(String::new()))]),
        )
        .unwrap();
        let text = String::from_utf8_lossy(&cleared);
        assert!(text.contains(r"\newcommand{\thesismonth}{}"));
        assert!(text.contains(r"\newcommand{\thesisyear}{}"));
        assert_eq!(
            bind_single_source_values(&cleared, &values).unwrap(),
            cleared
        );
        let legacy = br"% Known institutional single-source bindings.
\renewcommand{\thesismonth}{}
\renewcommand{\thesisyear}{}
% LATEX_CORE_SINGLE_SOURCE_BINDINGS";
        assert_eq!(
            single_source_managed_values_from_tex(legacy)["submission_date"],
            ""
        );
    }

    #[test]
    fn course_catalog_and_canonical_source_are_exact_and_idempotent() {
        let source = br"\documentclass{article}
\newcommand{\coursecode}{OLD}
\newcommand{\coursename}{Historical name} % preserve comment
\begin{document}\coursecode\space\coursename\end{document}";
        for (code, expected) in COURSE_CATALOG {
            assert_eq!(
                canonical_course_name(&format!(" {} ", code.to_lowercase())),
                Some(*expected)
            );
            let values = BTreeMap::from([
                ("course_code".into(), Value::String((*code).into())),
                ("course_name".into(), Value::String((*expected).into())),
            ]);
            let saved = bind_single_source_values(source, &values).unwrap();
            let text = String::from_utf8_lossy(&saved);
            assert!(text.contains(&format!("\\newcommand{{\\coursecode}}{{{code}}}")));
            assert!(text.contains(&format!("\\newcommand{{\\coursename}}{{{expected}}}")));
            assert!(text.contains("% preserve comment"));
            assert_eq!(text.matches(r"\newcommand{\coursecode}").count(), 1);
            assert_eq!(text.matches(r"\newcommand{\coursename}").count(), 1);
            for _ in 0..3 {
                assert_eq!(bind_single_source_values(&saved, &values).unwrap(), saved);
            }
            let reopened = single_source_values_from_tex(&saved);
            assert_eq!(reopened["course_code"], *code);
            assert_eq!(reopened["course_name"], *expected);
            assert_eq!(
                document_course_name(code, &reopened, &BTreeMap::new(), &values),
                Some((*expected).into())
            );
        }
        assert_eq!(canonical_course_name("UNKNOWN"), None);
        assert_eq!(canonical_course_name("BCSE497"), None);
    }

    #[test]
    fn edited_course_name_survives_only_for_the_same_course() {
        for (code, _) in COURSE_CATALOG {
            let saved = BTreeMap::from([
                ("course_code".into(), serde_json::json!(code)),
                ("course_name".into(), serde_json::json!("Intentional name")),
            ]);
            assert_eq!(
                document_course_name(code, &saved, &BTreeMap::new(), &saved),
                Some("Intentional name".into())
            );
        }
        let historical = BTreeMap::from([
            ("course_code".into(), serde_json::json!("BCSE497J")),
            (
                "course_name".into(),
                serde_json::json!("Intentional undergraduate name"),
            ),
        ]);
        assert_eq!(
            document_course_name("MACSE698", &historical, &historical, &historical),
            Some("Internship-I/Dissertation-I".into())
        );
        assert_eq!(
            document_course_name("BCSE4973", &historical, &historical, &historical),
            Some("Project-I".into())
        );
        assert_eq!(
            document_course_name("UNKNOWN", &historical, &historical, &historical),
            None
        );
        let manual = BTreeMap::from([
            ("course_code".into(), serde_json::json!("UNKNOWN")),
            (
                "course_name".into(),
                serde_json::json!("Manual unknown course"),
            ),
        ]);
        assert_eq!(
            document_course_name("UNKNOWN", &manual, &BTreeMap::new(), &manual),
            Some("Manual unknown course".into())
        );
    }

    #[test]
    fn registration_tuple_is_authoritative_and_missing_or_ambiguous_never_defaults() {
        for (raw, display) in [
            ("FALL", "Fall Semester"),
            ("Fall", "Fall Semester"),
            ("Fall Semester", "Fall Semester"),
            ("WINTER", "Winter Semester"),
            ("Winter", "Winter Semester"),
            ("Winter Semester", "Winter Semester"),
        ] {
            let (values, warnings) = registration_document_values(&[(
                "BCSE497J".into(),
                "2026-2027".into(),
                raw.into(),
            )]);
            assert!(warnings.is_empty());
            assert_eq!(values["course_code"], "BCSE497J");
            assert_eq!(values["team.academic_year"], "2026-2027");
            assert_eq!(values["team.semester"], display);
        }
        let (values, warnings) = registration_document_values(&[(
            " bcse497j ".into(),
            "2026-2027".into(),
            "Fall".into(),
        )]);
        assert!(warnings.is_empty());
        assert_eq!(
            values["course_code"], " bcse497j ",
            "lookup normalization must never replace the actual institutional identifier"
        );
        let (values, warnings) = registration_document_values(&[]);
        assert!(values.values().all(Value::is_null));
        assert!(warnings[0].contains("no applicable"));
        let (values, warnings) = registration_document_values(&[
            ("BCSE497J".into(), "2026-2027".into(), "FALL".into()),
            ("MACSE698".into(), "2026-2027".into(), "FALL".into()),
        ]);
        assert!(values.values().all(Value::is_null));
        assert!(warnings[0].contains("multiple applicable"));
        let (values, warnings) = registration_document_values(&[(
            "unknown ".into(),
            "2026-2027".into(),
            "SUMMER".into(),
        )]);
        assert_eq!(values["course_code"], "unknown ");
        assert_eq!(values["team.semester"], "SUMMER");
        assert!(warnings[0].contains("unsupported semester"));
    }

    #[test]
    fn complete_report_only_three_fields_are_document_owned() {
        for source in ["course_name", "project.title", "submission_date"] {
            assert!(document_owned_source(source));
        }
        for source in [
            "course_code",
            "team.semester",
            "team.academic_year",
            "guide.name",
            "degree_name",
        ] {
            assert!(!document_owned_source(source));
        }
    }

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
    fn complete_report_student_separators_preserve_names_and_registration_numbers() {
        let source =
            br"\studentAname  (\studentAregno), \studentBname (\studentBregno) \hspace{1cm}";
        let result = format_student_front_matter(source).unwrap();
        assert_eq!(&result[..], br"\studentAname\ (\studentAregno), \studentBname\ (\studentBregno) \setlength{\parindent}{1cm}\indent");
        assert_eq!(format_student_front_matter(&result).unwrap(), result);
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
    fn old_duplicate_bindings_normalize_without_removing_user_code() {
        let source = br"\documentclass{article}
\newcommand{\coursecode}{OLD}
\newcommand{\coursename}{Captsone project - II} % keep original comment
\newcommand{\programdegree}{Bachelor of Technology}
\newcommand{\custom}{original}
\renewcommand{\custom}{User value}
% Known institutional single-source bindings.
\providecommand{\coursecode}{}
\renewcommand{\coursecode}{BA101}
\providecommand{\coursename}{}
\renewcommand{\coursename}{Captsone project - II}
% keep user comment in managed area
\newcommand{\anothercustom}{keep me}
% LATEX_CORE_SINGLE_SOURCE_BINDINGS
\begin{document}\coursecode\end{document}
";
        let intended = BTreeMap::from([("course_code".into(), Value::String("BA102".into()))]);
        let mut bound = bind_single_source_values(source, &intended).unwrap();
        for _ in 0..3 {
            let text = std::str::from_utf8(&bound).unwrap();
            assert!(text.contains(r"\newcommand{\coursecode}{BA102}"));
            for command in ["coursecode", "coursename", "programdegree", "academicyear"] {
                assert_eq!(
                    text.matches(&format!("\\newcommand{{\\{command}}}"))
                        .count(),
                    1
                );
                assert!(!text.contains(&format!("\\renewcommand{{\\{command}}}")));
                assert!(!text.contains(&format!("\\providecommand{{\\{command}}}")));
            }
            assert!(text.contains(r"\renewcommand{\custom}{User value}"));
            assert!(text.contains("% keep original comment"));
            assert!(text.contains("% keep user comment in managed area"));
            assert!(text.contains(r"\newcommand{\anothercustom}{keep me}"));
            let next = bind_single_source_values(&bound, &intended).unwrap();
            assert_eq!(bound, next);
            bound = next;
        }
        let unchanged = bind_single_source_values(source, &BTreeMap::new()).unwrap();
        assert_eq!(
            single_source_values_from_tex(&unchanged)["course_code"],
            "BA101"
        );
    }

    #[test]
    fn original_multiline_definition_is_updated_in_place() {
        let source = b"\\documentclass{article}\n\\newcommand{\\coursecode}\n  {Old\nvalue} % retain\n\\begin{document}body\\end{document}\n";
        let bound = bind_single_source_values(
            source,
            &BTreeMap::from([("course_code".into(), Value::String("BA101".into()))]),
        )
        .unwrap();
        assert!(
            String::from_utf8_lossy(&bound)
                .contains("\\newcommand{\\coursecode}\n  {BA101} % retain")
        );
        assert!(!String::from_utf8_lossy(&bound).contains(r"\renewcommand{\coursecode}"));
    }

    #[test]
    fn student_slots_follow_authoritative_team_size() {
        for size in [1, 2, 4] {
            for (index, slot) in ["a", "b", "c", "d"].iter().enumerate() {
                assert_eq!(
                    legacy::unused_student_slot(&format!("student.{slot}.name"), size),
                    index >= size
                );
                assert_eq!(
                    legacy::unused_student_slot(&format!("student.{slot}.reg_no"), size),
                    index >= size
                );
            }
            assert!(!legacy::unused_student_slot("department_name", size));
        }
    }

    #[test]
    fn single_source_updates_preserve_manual_macros_and_crlf() {
        let main = b"\\documentclass{article}\r\n\\newcommand{\\thesistitle}{Manual \\LaTeX{} title}\r\n% manual preamble\r\n\\begin{document}body\\end{document}\r\n";
        let values = BTreeMap::from([("course_name".into(), Value::String("A new course".into()))]);
        let saved = bind_single_source_values(main, &values).unwrap();
        assert!(String::from_utf8_lossy(&saved).starts_with("\\documentclass{article}\r\n\\newcommand{\\thesistitle}{Manual \\LaTeX{} title}\r\n% manual preamble\r\n"));
        assert!(!String::from_utf8_lossy(&saved).contains(r"\renewcommand{\thesistitle}"));
        let managed = single_source_managed_values_from_tex(&saved);
        assert!(!managed.contains_key("project.title"));
        assert_eq!(managed["course_name"], "A new course");
        assert_eq!(bind_single_source_values(&saved, &managed).unwrap(), saved);
        assert_eq!(
            single_source_commands(&String::from_utf8_lossy(&saved)),
            legacy::commands(&String::from_utf8_lossy(main))
        );
    }

    #[test]
    fn single_source_persisted_bindings_roundtrip_without_accumulation() {
        let main = br"\documentclass{article}
\newcommand{\thesistitle}{Manual original}
% unrelated manual preamble
\begin{document}User body\end{document}
";
        let values = BTreeMap::from([
            (
                "project.title".into(),
                Value::String("Professor & durable title".into()),
            ),
            ("submission_date".into(), Value::String("2026-10-07".into())),
            ("team.semester".into(), Value::String("3".into())),
        ]);
        let saved = bind_single_source_values(main, &values).unwrap();
        assert!(String::from_utf8_lossy(&saved).contains("Professor \\& durable title"));
        assert!(String::from_utf8_lossy(&saved).contains("% unrelated manual preamble"));
        assert!(String::from_utf8_lossy(&saved).contains("User body"));
        let readback = single_source_values_from_tex(&saved);
        for (key, value) in &values {
            assert_eq!(readback.get(key), Some(value));
        }
        assert_eq!(bind_single_source_values(&saved, &readback).unwrap(), saved);
        let mut changed = readback;
        changed.insert("project.title".into(), Value::String("Second title".into()));
        let second = bind_single_source_values(&saved, &changed).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&second)
                .matches("% Known institutional single-source bindings.")
                .count(),
            1
        );
        assert_eq!(
            single_source_values_from_tex(&second)["project.title"],
            "Second title"
        );
    }

    #[test]
    fn single_source_binding_supports_marker_and_safe_automatic_mode_without_mutation() {
        let source = b"\\documentclass{article}\n\\newcommand{\\thesistitle}{Placeholder}\n% LATEX_CORE_SINGLE_SOURCE_BINDINGS\n\\begin{document}\n\\thesistitle\n\\end{document}\n";
        assert!(single_source_compatible(source));
        let mut values = BTreeMap::new();
        values.insert("project.title".into(), Value::String("A & B_%".into()));
        let bound = bind_single_source_values(source, &values).unwrap();
        let text = std::str::from_utf8(&bound).unwrap();
        assert_eq!(text.matches("LATEX_CORE_SINGLE_SOURCE_BINDINGS").count(), 1);
        assert!(text.contains(r"\newcommand{\thesistitle}{A \& B\_\%}"));
        assert_eq!(source, b"\\documentclass{article}\n\\newcommand{\\thesistitle}{Placeholder}\n% LATEX_CORE_SINGLE_SOURCE_BINDINGS\n\\begin{document}\n\\thesistitle\n\\end{document}\n");
        let automatic = b"\\documentclass{article}\n\\newcommand{\\thesistitle}{Placeholder}\n\\newcommand{\\studentAname}{Student}\n\\begin{document}\n\\thesistitle\n\\end{document}\n";
        assert_eq!(
            single_source_binding_point(automatic),
            Some(SingleSourceBindingPoint::AutomaticBeforeDocument)
        );
        let automatic_bound = bind_single_source_values(automatic, &BTreeMap::new()).unwrap();
        let automatic_text = std::str::from_utf8(&automatic_bound).unwrap();
        assert!(automatic_text.contains("% Known institutional single-source bindings."));
        assert!(automatic_text.contains(r"\newcommand{\thesistitle}{Placeholder}"));
        assert!(!automatic_text.contains(r"\renewcommand{\thesistitle}"));
        assert!(
            automatic_text
                .find("% Known institutional single-source bindings.")
                .unwrap()
                < automatic_text.find("\\begin{document}").unwrap()
        );
        assert!(!single_source_compatible(
            b"% LATEX_CORE_SINGLE_SOURCE_BINDINGS\n\\begin{document}\n"
        ));
        assert!(!single_source_compatible(b"\\newcommand{\\thesistitle}{x}\n\\begin{document}\n% LATEX_CORE_SINGLE_SOURCE_BINDINGS\n"));
        assert!(!single_source_compatible(b"\\newcommand{\\thesistitle}{x}\n% LATEX_CORE_SINGLE_SOURCE_BINDINGS\n% LATEX_CORE_SINGLE_SOURCE_BINDINGS\n\\begin{document}\n"));
        assert!(!single_source_compatible(
            b"\\newcommand{\\thesistitle}{x}\n\\begin{document}\n\\begin{document}\n"
        ));
        assert!(!single_source_compatible(
            b"\\newcommand{\\thesistitle}{x}\n\\begin{document}\\begin{document}\n"
        ));
        assert!(!single_source_compatible(
            b"\\newcommandish{\\thesistitle}{x}\n\\begin{document}\n"
        ));
        assert!(single_source_compatible(
            b"\\newcommand{\\thesistitle}{x}\n% \\begin{document}\n\\begin{document}\n"
        ));
    }

    #[test]
    fn synthetic_complete_report_fixture_keeps_body_and_assets_in_compile_only_binding() {
        let source = include_bytes!("../tests/fixtures/vit-complete-report/project.tex");
        assert!(single_source_compatible(source));
        let original = String::from_utf8(source.to_vec()).unwrap();
        let bound = bind_single_source_values(source, &BTreeMap::new()).unwrap();
        let bound = String::from_utf8(bound.to_vec()).unwrap();
        assert_eq!(original, String::from_utf8(source.to_vec()).unwrap());
        assert!(bound.contains(r"\input{chapters/body.tex}"));
        assert!(bound.contains(r"\input{acronym.tex}"));
        assert!(
            bound
                .find("% Known institutional single-source bindings.")
                .unwrap()
                < bound.find(r"\begin{document}").unwrap()
        );
    }

    #[test]
    fn professor_demo_assets_match_the_release_contract() {
        let pack = validate_archive(fixture(
            include_str!("../../../examples/professor-demo/front-matter/frontmatter.json"),
            &[
                (
                    "frontmatter.tex",
                    include_str!("../../../examples/professor-demo/front-matter/frontmatter.tex"),
                ),
                (
                    "cover.tex",
                    include_str!("../../../examples/professor-demo/front-matter/cover.tex"),
                ),
                (
                    "certificate.tex",
                    include_str!("../../../examples/professor-demo/front-matter/certificate.tex"),
                ),
                (
                    "declaration.tex",
                    include_str!("../../../examples/professor-demo/front-matter/declaration.tex"),
                ),
                (
                    "acknowledgements.tex",
                    include_str!(
                        "../../../examples/professor-demo/front-matter/acknowledgements.tex"
                    ),
                ),
                (
                    "abstract.tex",
                    include_str!("../../../examples/professor-demo/front-matter/abstract.tex"),
                ),
            ],
        ))
        .unwrap();
        assert_eq!(pack.manifest.sections.len(), 5);
        assert!(main_template_compatible(include_bytes!(
            "../../../examples/professor-demo/main-template/main.tex"
        )));
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
        let definitions = std::str::from_utf8(
            &rendered
                .files
                .iter()
                .find(|file| file.path.as_str().ends_with("/Front-Matter.tex"))
                .unwrap()
                .bytes,
        )
        .unwrap();
        assert!(definitions.contains("canonical: title; origin: AUTO"));
        assert!(definitions.contains(r"\csname LatexCoreTitle\endcsname"));
        assert!(!definitions.contains(r"\input{/etc/passwd}"));
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
