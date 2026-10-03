//! Explicit VIT metadata compatibility. Imported TeX is never substituted.
use super::{
    BTreeMap, BTreeSet, Bytes, FrontMatterError, FrontMatterField, FrontMatterFieldType,
    FrontMatterManifest, FrontMatterSection, ImportedFile, LogicalPath, MANAGED_ROOT,
    MANIFEST_PATH, RenderedFile, ResolvedValue, ValidatedPack, Value, empty_value,
    escape_latex_text, is_application_owned_path, resolve_and_render, valid_date,
};
use std::fmt::Write as _;

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct SemesterMetadata {
    pub label: String,
    pub academic_year_start: u32,
    pub academic_year_end: u32,
    pub academic_year: String,
    pub academic_year_display: String,
}

/// Resolve the institutional semester number and submission date into the
/// values consumed by Front Matter macros. The stored academic year remains
/// full-width; TeX formatting is applied only when bindings are emitted.
pub fn derive_semester_metadata(
    semester: &str,
    submission_date: &str,
    existing_academic_year: Option<&str>,
) -> Result<SemesterMetadata, FrontMatterError> {
    let semester = semester
        .parse::<u8>()
        .ok()
        .filter(|semester| (1..=8).contains(semester))
        .ok_or_else(|| FrontMatterError::InvalidValue("team.semester".into()))?;
    if !valid_date(submission_date) {
        return Err(FrontMatterError::InvalidValue("submission_date".into()));
    }
    let submission_year = submission_date[0..4]
        .parse::<u32>()
        .map_err(|_| FrontMatterError::InvalidValue("submission_date".into()))?;
    let (label, start, end) = if semester % 2 == 1 {
        ("Winter Semester", submission_year, submission_year + 1)
    } else {
        let start = submission_year
            .checked_sub(1)
            .ok_or_else(|| FrontMatterError::InvalidValue("submission_date".into()))?;
        ("Fall Semester", start, submission_year)
    };
    let academic_year = format!("{start}-{end}");
    if let Some(existing) = existing_academic_year.filter(|value| !value.trim().is_empty())
        && !matches!(existing, value if value == academic_year
            || value == format!("{start}-{:02}", end % 100)
            || value == format!("{start}--{}", end % 100)
            || value == format!("{start}\u{2013}{:02}", end % 100))
    {
        return Err(FrontMatterError::InvalidValue(
            "team.academic_year contradicts team.semester and submission_date".into(),
        ));
    }
    Ok(SemesterMetadata {
        label: label.into(),
        academic_year_start: start,
        academic_year_end: end,
        academic_year: academic_year.clone(),
        academic_year_display: format!("{start}\u{2013}{:02}", end % 100),
    })
}

fn apply_semester_metadata(
    values: &mut BTreeMap<String, ResolvedValue>,
) -> Result<(), FrontMatterError> {
    let Some(semester) = values
        .get("team.semester")
        .and_then(|value| value.value.as_str())
    else {
        return Ok(());
    };
    let Some(submission_date) = values
        .get("submission_date")
        .and_then(|value| value.value.as_str())
    else {
        return Ok(());
    };
    let metadata = derive_semester_metadata(
        semester,
        submission_date,
        values
            .get("team.academic_year")
            .and_then(|value| value.value.as_str()),
    )?;
    values.insert(
        "team.semester".into(),
        ResolvedValue {
            value: Value::String(metadata.label),
            source: "DERIVED",
        },
    );
    values.insert(
        "team.academic_year".into(),
        ResolvedValue {
            value: Value::String(metadata.academic_year),
            source: "DERIVED",
        },
    );
    Ok(())
}

pub const REGISTRY: &[(&str, &str, &str, bool)] = &[
    ("coursecode", "course_code", "Course code", true),
    ("coursename", "course_name", "Course name", true),
    ("thesistitle", "project.title", "Project title", true),
    ("thesismonth", "submission_date", "Submission date", true),
    ("thesisyear", "submission_date", "Submission date", true),
    ("teamsize", "team.size", "Team size", false),
    ("studentAname", "student.a.name", "Student A name", false),
    (
        "studentAregno",
        "student.a.reg_no",
        "Student A registration number",
        false,
    ),
    ("studentBname", "student.b.name", "Student B name", false),
    (
        "studentBregno",
        "student.b.reg_no",
        "Student B registration number",
        false,
    ),
    ("studentCname", "student.c.name", "Student C name", false),
    (
        "studentCregno",
        "student.c.reg_no",
        "Student C registration number",
        false,
    ),
    ("studentDname", "student.d.name", "Student D name", false),
    (
        "studentDregno",
        "student.d.reg_no",
        "Student D registration number",
        false,
    ),
    ("projguidename", "guide.name", "Project guide", false),
    (
        "projguidedesignation",
        "guide.designation",
        "Guide designation",
        false,
    ),
    ("schoolname", "school_name", "School name", false),
    ("programdegree", "degree_name", "Degree display name", false),
    (
        "programname",
        "programme_name",
        "Programme display name",
        false,
    ),
    ("specialization", "specialization", "Specialization", false),
    ("semester", "team.semester", "Semester", true),
    ("academicyear", "team.academic_year", "Academic Year", true),
    ("deanname", "dean.name", "Dean name", true),
    ("hodname", "hod.name", "HOD name", true),
    ("hoddept", "department_name", "Department name", false),
];

const SECTIONS: &[(&str, &[&str])] = &[
    ("cover", &["coverpage.tex", "cover.tex"]),
    ("certificate", &["certificate.tex"]),
    ("declaration", &["declaration.tex"]),
    (
        "acknowledgement",
        &["acknowledgement.tex", "acknowledgements.tex"],
    ),
];

// TeX control symbols consume one character. Thus \% is literal, while \\%
// ends in a comment. Control words contain ASCII letters under normal catcodes.
pub fn commands(source: &str) -> BTreeSet<String> {
    let mut chars = source.chars().peekable();
    let mut found = BTreeSet::new();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            for next in chars.by_ref() {
                if next == '\n' {
                    break;
                }
            }
        } else if ch == '\\' {
            let mut word = String::new();
            while chars.peek().is_some_and(char::is_ascii_alphabetic) {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            if word.is_empty() {
                chars.next();
            } else {
                found.insert(word);
            }
        }
    }
    found
}

fn metadata_like(word: &str) -> bool {
    [
        "student",
        "thesis",
        "team",
        "projguide",
        "hod",
        "dean",
        "school",
        "program",
        "course",
    ]
    .iter()
    .any(|prefix| word.starts_with(prefix))
        || matches!(word, "semester" | "specialization" | "academicyear")
}

pub fn referenced(files: &[ImportedFile]) -> Result<BTreeSet<String>, FrontMatterError> {
    let mut found = BTreeSet::new();
    for file in files
        .iter()
        .filter(|file| file.path.extension() == Some("tex"))
    {
        let source =
            std::str::from_utf8(&file.bytes).map_err(|_| FrontMatterError::InvalidManifest)?;
        found.extend(
            commands(source)
                .into_iter()
                .filter(|word| metadata_like(word)),
        );
    }
    Ok(found)
}

pub fn normalize(files: &[ImportedFile]) -> Result<FrontMatterManifest, FrontMatterError> {
    let mut sections = Vec::new();
    for (key, aliases) in SECTIONS {
        let matches: Vec<_> = files
            .iter()
            .filter(|file| aliases.contains(&file.path.as_str()))
            .collect();
        if matches.len() > 1 {
            return Err(FrontMatterError::DuplicateKey((*key).into()));
        }
        if let Some(file) = matches.first() {
            sections.push(FrontMatterSection {
                key: (*key).into(),
                label: (*key).into(),
                file: file.path.to_string(),
                required: false,
                default_enabled: true,
            });
        }
    }
    if sections.is_empty() {
        return Err(FrontMatterError::MissingManifest);
    }
    // The legacy renderer materializes generated metadata.tex under MANAGED_ROOT;
    // retaining a root metadata.tex would collide with that generated file.
    if files.iter().any(|file| {
        file.path.as_str() == "metadata.tex" || is_application_owned_path(file.path.as_str())
    }) {
        return Err(FrontMatterError::InvalidPath(
            "reserved generated Front Matter path".into(),
        ));
    }
    let referenced = referenced(files)?;
    let mut fields = BTreeMap::new();
    for (command, source, label, manual) in REGISTRY {
        if !referenced.contains(*command) {
            continue;
        }
        let key = source.replace('.', "_");
        fields.entry(key.clone()).or_insert(FrontMatterField {
            key,
            label: (*label).into(),
            field_type: if *source == "submission_date" {
                FrontMatterFieldType::Date
            } else {
                FrontMatterFieldType::Text
            },
            required: false,
            source: Some((*source).into()),
            default: None,
            allow_team_override: *manual,
        });
    }
    if referenced.contains("projguidename") || referenced.contains("projguidedesignation") {
        fields.insert(
            "guide_identity".into(),
            FrontMatterField {
                key: "guide_identity".into(),
                label: "Project guide selection".into(),
                field_type: FrontMatterFieldType::Text,
                required: false,
                source: None,
                default: None,
                allow_team_override: true,
            },
        );
    }
    if referenced.contains("deanname") {
        fields.insert(
            "dean_identity".into(),
            FrontMatterField {
                key: "dean_identity".into(),
                label: "Dean selection".into(),
                field_type: FrontMatterFieldType::Text,
                required: false,
                source: None,
                default: None,
                allow_team_override: true,
            },
        );
    }
    Ok(FrontMatterManifest {
        schema_version: 2,
        entry_file: "frontmatter.tex".into(),
        sections,
        fields: fields.into_values().collect(),
    })
}

pub fn warnings(pack: &ValidatedPack) -> Result<Vec<String>, FrontMatterError> {
    let mut warnings = Vec::new();
    for (section, _) in SECTIONS {
        if !pack
            .manifest
            .sections
            .iter()
            .any(|item| item.key == *section)
        {
            warnings.push(format!("This Front Matter pack has no {section} section."));
        }
    }
    for word in referenced(&pack.files)? {
        if !REGISTRY.iter().any(|item| item.0 == word) {
            warnings.push(format!(
                "Unmapped Front Matter field: \\{word}. It will be blank."
            ));
        }
    }
    if pack
        .files
        .iter()
        .find(|file| file.path.as_str() == "frontmatter.tex")
        .and_then(|file| std::str::from_utf8(&file.bytes).ok())
        .is_some_and(|source| {
            ["\\documentclass", "\\begin{document}", "\\end{document}"]
                .iter()
                .all(|marker| source.contains(marker))
        })
    {
        warnings.push(
            "Root frontmatter.tex is a standalone document; composition with a Main Content Template requires the separate composition path.".into(),
        );
    }
    Ok(warnings)
}

pub fn render(
    pack: &ValidatedPack,
    values: &BTreeMap<String, ResolvedValue>,
    enabled: &BTreeMap<String, bool>,
) -> Result<Vec<RenderedFile>, FrontMatterError> {
    let mut values = values.clone();
    apply_semester_metadata(&mut values)?;
    let mut metadata = String::from("% Generated by LaTeX Core.\n");
    let mut words = referenced(&pack.files)?;
    words.extend(REGISTRY.iter().map(|item| item.0.to_owned()));
    for word in words {
        writeln!(metadata, "\\providecommand{{\\{word}}}{{}}")
            .map_err(|_| FrontMatterError::InvalidManifest)?;
        let Some((_, source, _, _)) = REGISTRY.iter().find(|item| item.0 == word) else {
            continue;
        };
        let Some(value) = values
            .get(&source.replace('.', "_"))
            .and_then(|value| value.value.as_str())
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let value = match word.as_str() {
            "thesismonth" if valid_date(value) => [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ][value[5..7]
                .parse::<usize>()
                .map_err(|_| FrontMatterError::InvalidValue("submission_date".into()))?
                - 1]
            .to_owned(),
            "thesisyear" if valid_date(value) => value[..4].to_owned(),
            "academicyear" if value.len() == 9 && value.as_bytes()[4] == b'-' => {
                format!("{}--{}", &value[..4], &value[7..9])
            }
            _ => value.to_owned(),
        };
        writeln!(
            metadata,
            "\\renewcommand{{\\{word}}}{{{}}}",
            escape_latex_text(&value)
        )
        .map_err(|_| FrontMatterError::InvalidManifest)?;
    }
    let mut wrapper = format!("\\input{{{MANAGED_ROOT}/metadata.tex}}\n");
    for section in &pack.manifest.sections {
        if enabled[&section.key] {
            writeln!(wrapper, "\\input{{{MANAGED_ROOT}/{}}}", section.file)
                .map_err(|_| FrontMatterError::InvalidManifest)?;
        }
    }
    let mut files = pack
        .files
        .iter()
        .filter(|file| file.path.as_str() != MANIFEST_PATH)
        .map(|file| {
            Ok(RenderedFile {
                path: LogicalPath::parse(&format!("{MANAGED_ROOT}/{}", file.path))
                    .map_err(|_| FrontMatterError::InvalidPath(file.path.to_string()))?,
                bytes: file.bytes.clone(),
            })
        })
        .collect::<Result<Vec<_>, FrontMatterError>>()?;
    for (path, bytes) in [("metadata.tex", metadata), ("frontmatter.tex", wrapper)] {
        files.push(RenderedFile {
            path: LogicalPath::parse(&format!("{MANAGED_ROOT}/{path}"))
                .map_err(|_| FrontMatterError::InvalidPath(path.into()))?,
            bytes: Bytes::from(bytes),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Bind only the explicitly allowlisted institutional zero-argument macros.
/// The single-source template must declare its placeholders before the verified
/// marker; this generated file is report-scoped and never changes the template.
pub fn single_source_bindings(
    values: &BTreeMap<String, Value>,
) -> Result<String, FrontMatterError> {
    let mut values = values.clone();
    if let (Some(semester), Some(submission_date)) = (
        values.get("team.semester").and_then(Value::as_str),
        values.get("submission_date").and_then(Value::as_str),
    ) {
        let metadata = derive_semester_metadata(
            semester,
            submission_date,
            values.get("team.academic_year").and_then(Value::as_str),
        )?;
        values.insert("team.semester".into(), Value::String(metadata.label));
        values.insert(
            "team.academic_year".into(),
            Value::String(metadata.academic_year),
        );
    }
    let mut output = String::from("% Known institutional single-source bindings.\n");
    for (word, source, _, _) in REGISTRY {
        writeln!(output, "\\providecommand{{\\{word}}}{{}}")
            .map_err(|_| FrontMatterError::InvalidManifest)?;
        let Some(value) = values
            .get(*source)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let value = match *word {
            "thesismonth" if valid_date(value) => [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ][value[5..7]
                .parse::<usize>()
                .map_err(|_| FrontMatterError::InvalidValue("submission_date".into()))?
                - 1]
            .to_owned(),
            "thesisyear" if valid_date(value) => value[..4].to_owned(),
            "academicyear" if value.len() == 9 && value.as_bytes()[4] == b'-' => {
                format!("{}--{}", &value[..4], &value[7..9])
            }
            _ => value.to_owned(),
        };
        writeln!(
            output,
            "\\renewcommand{{\\{word}}}{{{}}}",
            escape_latex_text(&value)
        )
        .map_err(|_| FrontMatterError::InvalidManifest)?;
    }
    Ok(output)
}

/// Read-only status calculation: resolving data never creates a workspace version.
pub fn details(
    pack: &ValidatedPack,
    automatic: &BTreeMap<String, Value>,
    overrides: &BTreeMap<String, Value>,
    sections: &BTreeMap<String, bool>,
) -> Result<Value, FrontMatterError> {
    let rendered = resolve_and_render(pack, automatic, overrides, sections)?;
    let options = automatic
        .get("guide.options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut missing = Vec::new();
    let mut warnings = warnings(pack)?;
    let size = automatic
        .get("team.size")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if size == 0 {
        warnings.push("This Team has no Writers; student fields are blank.".into());
    }
    if size > 4 {
        warnings.push("This Front Matter template supports 4 student slots; additional Writers are not represented.".into());
    }
    let mut fields = Vec::new();
    for field in &pack.manifest.fields {
        let resolved = rendered.resolved.get(&field.key);
        let value = resolved.map(|value| &value.value);
        let has_value = value.is_some_and(|value| !empty_value(value));
        let team_metadata = matches!(
            field.source.as_deref(),
            Some("team.semester" | "team.academic_year")
        );
        let mut editable = (field.allow_team_override || team_metadata)
            && resolved.is_none_or(|value| value.source != "AUTO");
        let dean_options = automatic
            .get("dean.options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if field.key == "guide_identity" {
            editable = options.len() > 1;
        }
        if field.key == "dean_identity" {
            editable = dean_options.len() > 1;
        }
        if field.key == "dean_name" && dean_options.len() > 1 {
            editable = false;
        }
        if editable && !has_value {
            missing.push(field.label.clone());
        }
        // Unused student slots are intentionally empty, not missing institutional data.
        let unused_slot = ["a", "b", "c", "d"]
            .iter()
            .enumerate()
            .any(|(index, slot)| {
                index >= size && field.key.starts_with(&format!("student_{slot}_"))
            });
        if !has_value && !unused_slot && !field.key.ends_with("_identity") {
            warnings.push(format!(
                "{} is not available from institution data or Document details.",
                field.label
            ));
        }
        let mut descriptor =
            serde_json::to_value(field).map_err(|_| FrontMatterError::InvalidManifest)?;
        descriptor["editable"] = Value::Bool(editable);
        if field.key == "guide_identity" {
            descriptor["options"] = Value::Array(options.clone());
        }
        if field.key == "dean_identity" {
            descriptor["options"] = Value::Array(dean_options);
        }
        fields.push(descriptor);
    }
    if options.len() > 1 && overrides.get("guide_identity").is_none_or(empty_value) {
        warnings.push("Multiple Mentors are assigned; choose the project guide.".into());
    }
    Ok(
        serde_json::json!({"fields":fields,"missing":missing,"warnings":warnings,"values":rendered.resolved.iter().map(|(key,value)| serde_json::json!({"field_key":key,"value":value.value,"value_source":value.source})).collect::<Vec<_>>() }),
    )
}

/// A previously chosen identity may cease to apply after institutional changes.
/// Keep that historical choice in history, but require a current applicable choice.
pub fn applicable_overrides(
    automatic: &BTreeMap<String, Value>,
    overrides: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let mut result = overrides.clone();
    for identity in ["guide", "dean"] {
        let key = format!("{identity}_identity");
        let options = automatic
            .get(&format!("{identity}.options"))
            .and_then(Value::as_array);
        if result.get(&key).is_some_and(|chosen| {
            !options.is_some_and(|items| items.iter().any(|item| item["value"] == *chosen))
        }) {
            result.remove(&key);
        }
        if identity == "dean" && options.is_some_and(|items| items.len() > 1) {
            result.remove("dean_name");
        }
    }
    result
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "unit test fixtures contain fixed valid paths and values"
)]
mod tests {
    use super::*;
    use crate::{archive::ImportedArchive, front_matter::validate_archive};

    fn fixture() -> ValidatedPack {
        let files = [
            (
                "coverpage.tex",
                include_str!("../../tests/fixtures/vit-front-matter/coverpage.tex"),
            ),
            (
                "certificate.tex",
                include_str!("../../tests/fixtures/vit-front-matter/certificate.tex"),
            ),
            (
                "declaration.tex",
                include_str!("../../tests/fixtures/vit-front-matter/declaration.tex"),
            ),
            (
                "acknowledgement.tex",
                include_str!("../../tests/fixtures/vit-front-matter/acknowledgement.tex"),
            ),
        ]
        .into_iter()
        .map(|(path, text)| ImportedFile {
            path: LogicalPath::parse(path).unwrap(),
            bytes: Bytes::from(text),
        })
        .collect();
        validate_archive(ImportedArchive {
            files,
            detected_main: None,
        })
        .unwrap()
    }

    #[test]
    fn comments_control_symbols_and_registry() {
        let pack = fixture();
        let words = referenced(&pack.files).unwrap();
        for (word, _, _, _) in REGISTRY {
            assert!(words.contains(*word), "{word}");
        }
        for word in ["studentBgender", "studentBhodname", "projguidegender"] {
            assert!(!words.contains(word));
        }
        let words =
            commands("\\% \\thesistitle\n\\\\% \\studentBgender\n% \\projguidegender\n\\semester");
        assert!(words.contains("thesistitle"));
        assert!(words.contains("semester"));
        assert!(!words.contains("studentBgender"));
        assert!(!words.contains("projguidegender"));
    }

    #[test]
    fn real_vit_archive_with_root_frontmatter_is_preserved() {
        let files: Vec<ImportedFile> = [
            (
                "frontmatter.tex",
                include_bytes!("../../tests/fixtures/vit-front-matter/frontmatter.tex").as_slice(),
            ),
            (
                "VITSCOPEThesis.cls",
                include_bytes!("../../tests/fixtures/vit-front-matter/VITSCOPEThesis.cls")
                    .as_slice(),
            ),
            (
                "cover.tex",
                include_bytes!("../../tests/fixtures/vit-front-matter/cover.tex").as_slice(),
            ),
            (
                "certificate.tex",
                include_bytes!("../../tests/fixtures/vit-front-matter/certificate.tex").as_slice(),
            ),
            (
                "declaration.tex",
                include_bytes!("../../tests/fixtures/vit-front-matter/declaration.tex").as_slice(),
            ),
            (
                "acknowledgements.tex",
                include_bytes!("../../tests/fixtures/vit-front-matter/acknowledgements.tex")
                    .as_slice(),
            ),
            (
                "images/test-logo.png",
                include_bytes!("../../tests/fixtures/vit-front-matter/images/test-logo.png")
                    .as_slice(),
            ),
        ]
        .into_iter()
        .map(|(path, bytes)| ImportedFile {
            path: LogicalPath::parse(path).unwrap(),
            bytes: Bytes::copy_from_slice(bytes),
        })
        .collect();
        let root_source = files
            .iter()
            .find(|file| file.path.as_str() == "frontmatter.tex")
            .unwrap()
            .bytes
            .clone();
        let pack = validate_archive(ImportedArchive {
            files,
            detected_main: None,
        })
        .unwrap();

        assert_eq!(pack.manifest.schema_version, 2);
        assert_eq!(
            pack.manifest
                .sections
                .iter()
                .map(|section| section.key.as_str())
                .collect::<Vec<_>>(),
            ["cover", "certificate", "declaration", "acknowledgement"]
        );
        assert_eq!(
            pack.files
                .iter()
                .find(|file| file.path.as_str() == "frontmatter.tex")
                .unwrap()
                .bytes,
            root_source
        );
        assert_eq!(
            pack.files
                .iter()
                .find(|file| file.path.as_str() == "VITSCOPEThesis.cls")
                .unwrap()
                .bytes
                .as_ref(),
            include_bytes!("../../tests/fixtures/vit-front-matter/VITSCOPEThesis.cls")
        );
        assert_eq!(
            pack.files
                .iter()
                .find(|file| file.path.as_str() == "images/test-logo.png")
                .unwrap()
                .bytes
                .as_ref(),
            include_bytes!("../../tests/fixtures/vit-front-matter/images/test-logo.png")
        );
        let words = referenced(&pack.files).unwrap();
        assert!(words.contains("thesistitle"));
        assert!(words.contains("projguidename"));
        assert!(
            warnings(&pack)
                .unwrap()
                .iter()
                .any(|warning| warning.contains("standalone document"))
        );
    }

    #[test]
    fn managed_frontmatter_paths_and_root_metadata_remain_reserved() {
        let base = vec![
            ImportedFile {
                path: LogicalPath::parse("cover.tex").unwrap(),
                bytes: Bytes::from_static(b"cover"),
            },
            ImportedFile {
                path: LogicalPath::parse("certificate.tex").unwrap(),
                bytes: Bytes::from_static(b"certificate"),
            },
        ];
        for reserved in [".latex-core/frontmatter/frontmatter.tex", "metadata.tex"] {
            let mut files = base.clone();
            files.push(ImportedFile {
                path: LogicalPath::parse(reserved).unwrap(),
                bytes: Bytes::from_static(b"reserved"),
            });
            assert!(matches!(
                validate_archive(ImportedArchive {
                    files,
                    detected_main: None,
                }),
                Err(FrontMatterError::InvalidPath(_))
            ));
        }
    }

    #[test]
    fn immutable_sections_safe_definitions_and_semantic_date() {
        let pack = fixture();
        let automatic = BTreeMap::from([
            (
                "team.name".into(),
                Value::String(r"A & % $ # _ { } \input \write18".into()),
            ),
            ("team.size".into(), Value::String("4".into())),
        ]);
        let overrides =
            BTreeMap::from([("submission_date".into(), Value::String("2026-09-13".into()))]);
        let rendered = resolve_and_render(&pack, &automatic, &overrides, &BTreeMap::new()).unwrap();
        for original in pack
            .files
            .iter()
            .filter(|file| file.path.extension() == Some("tex"))
        {
            assert_eq!(
                rendered
                    .files
                    .iter()
                    .find(|file| file.path.as_str().ends_with(original.path.as_str()))
                    .unwrap()
                    .bytes,
                original.bytes
            );
        }
        let metadata = std::str::from_utf8(
            &rendered
                .files
                .iter()
                .find(|file| file.path.as_str().ends_with("/metadata.tex"))
                .unwrap()
                .bytes,
        )
        .unwrap();
        assert!(metadata.contains(r"\renewcommand{\thesismonth}{September}"));
        assert!(metadata.contains(r"\renewcommand{\thesisyear}{2026}"));
        assert!(metadata.contains(r"\providecommand{\studentAname}{}"));
        assert!(!metadata.contains(r"\renewcommand{\studentAname}"));
        assert!(metadata.contains(&escape_latex_text(automatic["team.name"].as_str().unwrap())));
        assert!(!metadata.contains(r"\write18"));
        assert!(!metadata.contains("gender"));
    }

    #[test]
    fn single_source_bindings_are_allowlisted_and_escaped() {
        let values = BTreeMap::from([
            ("project.title".into(), Value::String("Safe & exact".into())),
            ("student.a.name".into(), Value::String("Alice Alpha".into())),
        ]);
        let bindings = single_source_bindings(&values).unwrap();
        assert!(bindings.contains("\\renewcommand{\\thesistitle}{Safe \\& exact}"));
        assert!(bindings.contains("\\renewcommand{\\studentAname}{Alice Alpha}"));
        assert!(!bindings.contains("write18"));
    }

    #[test]
    fn single_source_title_uses_source_until_project_title_is_explicit() {
        let source_only = single_source_bindings(&BTreeMap::from([(
            "team.name".into(),
            Value::String("Team-2".into()),
        )]))
        .unwrap();
        assert!(!source_only.contains(r"\renewcommand{\thesistitle}"));
        let explicit = single_source_bindings(&BTreeMap::from([(
            "project.title".into(),
            Value::String("Title B".into()),
        )]))
        .unwrap();
        assert!(explicit.contains(r"\renewcommand{\thesistitle}{Title B}"));
    }

    #[test]
    fn semester_metadata_uses_submission_year_and_rejects_invalid_numbers() {
        for (semester, date, expected_label, expected_year, expected_display) in [
            ("1", "2026-09-01", "Winter Semester", "2026-2027", "2026–27"),
            ("3", "2026-09-01", "Winter Semester", "2026-2027", "2026–27"),
            ("7", "2026-09-01", "Winter Semester", "2026-2027", "2026–27"),
            ("2", "2026-09-01", "Fall Semester", "2025-2026", "2025–26"),
            ("4", "2026-09-01", "Fall Semester", "2025-2026", "2025–26"),
            ("8", "2026-09-01", "Fall Semester", "2025-2026", "2025–26"),
            ("1", "2027-09-01", "Winter Semester", "2027-2028", "2027–28"),
            ("2", "2027-09-01", "Fall Semester", "2026-2027", "2026–27"),
        ] {
            let metadata = derive_semester_metadata(semester, date, None).unwrap();
            assert_eq!(metadata.label, expected_label);
            assert_eq!(metadata.academic_year, expected_year);
            assert_eq!(metadata.academic_year_display, expected_display);
        }
        assert!(derive_semester_metadata("0", "2026-09-01", None).is_err());
        assert!(derive_semester_metadata("9", "2026-09-01", None).is_err());
        assert!(derive_semester_metadata("4", "2026-09-01", Some("2025-26")).is_ok());
        assert!(derive_semester_metadata("1", "2026-09-01", Some("2025-2026")).is_err());
    }

    #[test]
    fn semester_bindings_keep_submission_year_separate_from_academic_year() {
        let bindings = single_source_bindings(&BTreeMap::from([
            ("team.semester".into(), Value::String("4".into())),
            ("submission_date".into(), Value::String("2026-09-01".into())),
        ]))
        .unwrap();
        assert!(bindings.contains(r"\renewcommand{\semester}{Fall Semester}"));
        assert!(bindings.contains(r"\renewcommand{\academicyear}{2025--26}"));
        assert!(bindings.contains(r"\renewcommand{\thesisyear}{2026}"));
    }

    #[test]
    fn missing_sections_unknown_metadata_and_writer_sizes_are_nonfatal() {
        let mut pack = fixture();
        pack.files
            .retain(|file| !matches!(file.path.as_str(), "frontmatter.json" | "certificate.tex"));
        pack.files.push(ImportedFile {
            path: LogicalPath::parse("extra.tex").unwrap(),
            bytes: Bytes::from_static(b"\\studentUnknown\\customfoo"),
        });
        let pack = validate_archive(ImportedArchive {
            files: pack.files,
            detected_main: None,
        })
        .unwrap();
        assert_eq!(warnings(&pack).unwrap().len(), 2);
        for size in 0..=5 {
            let automatic = BTreeMap::from([("team.size".into(), Value::String(size.to_string()))]);
            let detail = details(&pack, &automatic, &BTreeMap::new(), &BTreeMap::new()).unwrap();
            assert_eq!(
                detail["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value.as_str().unwrap().contains("additional Writers")),
                size > 4
            );
            let files = resolve_and_render(&pack, &automatic, &BTreeMap::new(), &BTreeMap::new())
                .unwrap()
                .files;
            let metadata = std::str::from_utf8(
                &files
                    .iter()
                    .find(|file| file.path.as_str().ends_with("metadata.tex"))
                    .unwrap()
                    .bytes,
            )
            .unwrap();
            assert!(metadata.contains(r"\providecommand{\studentUnknown}{}"));
            assert!(!metadata.contains("customfoo"));
        }
    }

    #[test]
    fn arbitrary_archive_invalid_dates_and_auto_overrides() {
        assert!(
            validate_archive(ImportedArchive {
                files: vec![ImportedFile {
                    path: LogicalPath::parse("main.tex").unwrap(),
                    bytes: Bytes::new()
                }],
                detected_main: None
            })
            .is_err()
        );
        let pack = fixture();
        assert!(
            resolve_and_render(
                &pack,
                &BTreeMap::new(),
                &BTreeMap::from([("submission_date".into(), Value::String("2026-02-30".into()))]),
                &BTreeMap::new()
            )
            .is_err()
        );
        let auto = BTreeMap::from([("hod.name".into(), Value::String("Canonical HOD".into()))]);
        let rendered = resolve_and_render(
            &pack,
            &auto,
            &BTreeMap::from([("hod_name".into(), Value::String("Old override".into()))]),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(rendered.resolved["hod_name"].source, "AUTO");
    }
}
