//! Explicit VIT metadata compatibility. Imported TeX is never substituted.
use super::{
    BTreeMap, BTreeSet, Bytes, FrontMatterError, FrontMatterField, FrontMatterFieldType,
    FrontMatterManifest, FrontMatterSection, ImportedFile, LogicalPath, MANAGED_ROOT,
    MANIFEST_PATH, RenderedFile, ResolvedValue, ValidatedPack, Value, empty_value,
    escape_latex_text, resolve_and_render, valid_date,
};
use std::fmt::Write as _;

pub const REGISTRY: &[(&str, &str, &str, bool)] = &[
    ("coursecode", "course_code", "Course code", true),
    ("coursename", "course_name", "Course name", true),
    ("thesistitle", "team.name", "Team title", false),
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
    ("schoolname", "school_name", "School name", true),
    ("programdegree", "degree_name", "Degree display name", true),
    (
        "programname",
        "programme_name",
        "Programme display name",
        true,
    ),
    ("specialization", "specialization", "Specialization", true),
    ("semester", "team.semester", "Semester", false),
    ("deanname", "dean.name", "Dean name", true),
    ("hodname", "hod.name", "HOD name", true),
    ("hoddept", "department_name", "Department name", true),
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
        || matches!(word, "semester" | "specialization")
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
    if files
        .iter()
        .any(|file| matches!(file.path.as_str(), "metadata.tex" | "frontmatter.tex"))
    {
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
    Ok(warnings)
}

pub fn render(
    pack: &ValidatedPack,
    values: &BTreeMap<String, ResolvedValue>,
    enabled: &BTreeMap<String, bool>,
) -> Result<Vec<RenderedFile>, FrontMatterError> {
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
                - 1],
            "thesisyear" if valid_date(value) => &value[..4],
            _ => value,
        };
        writeln!(
            metadata,
            "\\renewcommand{{\\{word}}}{{{}}}",
            escape_latex_text(value)
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
        let mut editable =
            field.allow_team_override && resolved.is_none_or(|value| value.source != "AUTO");
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
