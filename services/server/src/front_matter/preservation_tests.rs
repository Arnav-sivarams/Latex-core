#![allow(clippy::unwrap_used, reason = "isolated preservation test assertions")]

use super::*;

pub(crate) fn sensitive_source(newline: &str) -> String {
    [
        r"\documentclass{article}",
        r"\usepackage{geometry,setspace,titlesec}",
        r"\geometry{margin=25mm}",
        r"\newcommand* {\coursename} % definition comment",
        r"  {Old {nested} \{course\} \% name} % neighboring comment",
        r"\def\thesistitle{Original title}",
        r"\providecommand\thesismonth{October}",
        r"\newcommand{\thesisyear}[0]{2026}",
        r"\newcommand{\latexcoresubmissiondate}{2026-10-07}",
        r"\newcommand{\customspacing}{\linespread{1.3}\setstretch{1.5}}",
        r"\renewcommand{\baselinestretch}{1.2}",
        r"\providecommand{\customfont}{\sffamily\large}",
        r"\def\customskip{\vspace*{3mm}}",
        r"\setlength{\parskip}{6pt}",
        r"\setlength{\parindent}{0pt}",
        r"\setlength{\baselineskip}{15pt}",
        r"\titlespacing*{\section}{0pt}{12pt}{6pt}",
        r"% \newcommand{\coursename}{fake} \hspace{1cm} \vspace{5mm}",
        r"\begin{document}",
        r"\begin{titlepage}\customfont\thesistitle\end{titlepage}",
        r"\section{Certificate}\coursename\par",
        r"  \hspace{1cm} First\hspace*{2cm} Second\par",
        r"\vspace{5mm}\vspace*{2mm}\customspacing",
        r"\newpage\noindent Body \textbf{with {nested} braces}.",
        r"\end{document}",
        "",
    ]
    .join(newline)
}

#[test]
fn only_authorized_values_change_with_original_line_endings() {
    for newline in ["\n", "\r\n"] {
        let source = sensitive_source(newline);
        let values = BTreeMap::from([
            ("course_name".into(), Value::String("New & course_%".into())),
            ("project.title".into(), Value::String("New title".into())),
            ("submission_date".into(), Value::String("2027-02-09".into())),
        ]);
        let expected = source
            .replacen(r"Old {nested} \{course\} \% name", r"New \& course\_\%", 1)
            .replacen("Original title", "New title", 1)
            .replacen("{October}", "{February}", 1)
            .replacen("{2026}", "{2027}", 1)
            .replacen("{2026-10-07}", "{2027-02-09}", 1);
        let mut saved = bind_single_source_values(source.as_bytes(), &values).unwrap();
        assert_eq!(saved.as_ref(), expected.as_bytes());
        assert_eq!(
            single_source_values_from_tex(&saved)["course_name"],
            values["course_name"]
        );
        for _ in 0..3 {
            let next = bind_single_source_values(&saved, &values).unwrap();
            assert_eq!(next, saved);
            saved = next;
        }
    }
}

#[test]
fn empty_save_and_unchanged_literal_values_preserve_user_source() {
    let source = sensitive_source("\r\n");
    assert_eq!(
        bind_single_source_values(source.as_bytes(), &BTreeMap::new())
            .unwrap()
            .as_ref(),
        source.as_bytes()
    );
    let current = single_source_values_from_tex(source.as_bytes());
    assert_eq!(
        bind_single_source_values(source.as_bytes(), &current)
            .unwrap()
            .as_ref(),
        source.as_bytes()
    );
}

#[test]
fn existing_renewals_and_old_generated_blocks_are_not_rebuilt() {
    let source = "\\newcommand{\\coursename}{Original}\r\n% Known institutional single-source bindings.\r\n\\providecommand{\\coursename}{} % keep\r\n\\renewcommand{\\coursename}\r\n {Effective} % keep too\r\n\\def\\customspacing{\\hspace{1cm}}\r\n% LATEX_CORE_SINGLE_SOURCE_BINDINGS\r\n\\begin{document}body\\end{document}\r\n";
    let values = BTreeMap::from([("course_name".into(), Value::String("Updated".into()))]);
    let saved = bind_single_source_values(source.as_bytes(), &values).unwrap();
    assert_eq!(
        saved.as_ref(),
        source.replacen("{Effective}", "{Updated}", 1).as_bytes()
    );
    assert_eq!(bind_single_source_values(&saved, &values).unwrap(), saved);
}

#[test]
fn unsupported_managed_structures_fail_without_adding_duplicates() {
    for declaration in [
        r"\newcommand{\coursename}[1]{#1}",
        r"\def\coursename#1{#1}",
        r"\edef\coursename{Expanded}",
        r"\let\coursename\othername",
        r"\newcommand{\wrapper}{\newcommand{\coursename}{Nested}}",
        r"\newcommand{\coursename}{unclosed",
        "\\newcommand{\\coursename}{Old % inside value\n name}",
    ] {
        let source = format!(
            "\\newcommand{{\\thesistitle}}{{Title}}\n{declaration}\n\\begin{{document}}body\\end{{document}}\n"
        );
        let values = BTreeMap::from([("course_name".into(), Value::String("Updated".into()))]);
        let error = bind_single_source_values(source.as_bytes(), &values).unwrap_err();
        assert!(error.to_string().contains("coursename"), "{error}");
    }
}

#[test]
fn escaped_control_symbols_and_commented_fake_definitions_are_ignored() {
    let source = "\\newcommand{\\coursename}{Actual}\n% \\renewcommand{\\coursename}{comment}\n\\newcommand{\\custom}{\\\\renewcommand{\\coursename}{text}}\n\\begin{document}\\end{document}\n";
    let values = BTreeMap::from([("course_name".into(), Value::String("Updated".into()))]);
    assert_eq!(
        bind_single_source_values(source.as_bytes(), &values)
            .unwrap()
            .as_ref(),
        source.replacen("{Actual}", "{Updated}", 1).as_bytes()
    );
}

#[test]
fn generated_helpers_do_not_move_or_consume_an_indented_marker() {
    let source = "\\newcommand{\\thesistitle}{Title}\r\n% mention LATEX_CORE_SINGLE_SOURCE_BINDINGS inline\r\n  % LATEX_CORE_SINGLE_SOURCE_BINDINGS\r\n\\begin{document}\\end{document}\r\n";
    let values = BTreeMap::from([("submission_date".into(), Value::String("2026-10-07".into()))]);
    let saved = bind_single_source_values(source.as_bytes(), &values).unwrap();
    assert!(saved.ends_with(
        b"  % LATEX_CORE_SINGLE_SOURCE_BINDINGS\r\n\\begin{document}\\end{document}\r\n"
    ));
    assert!(saved.starts_with(b"\\newcommand{\\thesistitle}{Title}\r\n% mention LATEX_CORE_SINGLE_SOURCE_BINDINGS inline\r\n"));
    assert_eq!(bind_single_source_values(&saved, &values).unwrap(), saved);
}

#[test]
fn robust_definitions_keep_their_structure_and_body_definitions_are_rejected() {
    let source = "\\DeclareRobustCommand* \\coursename\n  {Old} % preserve\n\\begin{document}\\end{document}\n";
    let values = BTreeMap::from([("course_name".into(), Value::String("New".into()))]);
    assert_eq!(
        bind_single_source_values(source.as_bytes(), &values)
            .unwrap()
            .as_ref(),
        source.replacen("{Old}", "{New}", 1).as_bytes()
    );
    let body_definition = source.replacen(
        r"\end{document}",
        r"\renewcommand{\coursename}{Local}\end{document}",
        1,
    );
    assert!(
        bind_single_source_values(body_definition.as_bytes(), &values)
            .unwrap_err()
            .to_string()
            .contains("document body")
    );
}

#[test]
fn a_marker_inside_an_unmanaged_macro_is_not_an_insertion_point() {
    let source = "\\newcommand{\\coursename}{Course}\n\\newcommand{\\custom}{\n% LATEX_CORE_SINGLE_SOURCE_BINDINGS\nUntouched formatting \\hspace{1cm}\n}\n\\begin{document}\\end{document}\n";
    let values = BTreeMap::from([("submission_date".into(), Value::String("2026-10-07".into()))]);
    assert!(
        bind_single_source_values(source.as_bytes(), &values)
            .unwrap_err()
            .to_string()
            .contains("top level")
    );
}
