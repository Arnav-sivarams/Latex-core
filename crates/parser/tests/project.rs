#![allow(clippy::unwrap_used, reason = "tests use known-valid fixtures")]
use bytes::Bytes;
use core_types::LogicalPath;
use latex_parser::*;
use std::collections::BTreeMap;
fn p(v: &str) -> LogicalPath {
    LogicalPath::parse(v).unwrap()
}
fn project(entries: &[(&str, &'static [u8])]) -> ProjectSource {
    let files = entries
        .iter()
        .map(|(path, data)| (p(path), Bytes::from_static(data)))
        .collect();
    ProjectSource::new(p("main.tex"), files).unwrap()
}
#[test]
fn dependencies_tex_requests_and_determinism() {
    let source=project(&[("main.tex",br"\documentclass{vitthesis}\usepackage{styles/local,amsmath}\input{chapters/one}\includegraphics{figures/chart}\bibliography{references}"),("chapters/one.tex",b"text"),("styles/local.sty",b"text"),("vitthesis.cls",b"text"),("figures/chart.pdf",b"binary"),("references.bib",b"@book{x}")]);
    let analyzer = ProjectAnalyzer::with_default_limits();
    let a = analyzer.analyze(&source).unwrap();
    assert_eq!(a, analyzer.analyze(&source).unwrap());
    for raw in ["chapters/one", "figures/chart", "references"] {
        assert!(
            a.dependency_graph().edges().iter().any(|e| e.raw() == raw
                && matches!(e.resolution(), DependencyResolution::ProjectFile(_)))
        );
    }
    assert!(
        a.dependency_graph()
            .tex_requests()
            .iter()
            .any(|e| e.name() == "vitthesis"
                && matches!(e.resolution(), DependencyResolution::ProjectFile(_)))
    );
    assert!(
        a.dependency_graph()
            .tex_requests()
            .iter()
            .any(|e| e.name() == "amsmath" && e.resolution() == &DependencyResolution::ExternalTex)
    );
    assert!(!a.files().contains_key(&p("references.bib")));
}
#[test]
fn missing_dynamic_cycles_and_labels_are_diagnostics() {
    let source=project(&[("main.tex",br"\input{missing}\input{a}\input{\jobname-generated}\label{dup}\ref{ok}\ref{missing-ref}"),("a.tex",br"\input{b}\label{ok}\label{dup}"),("b.tex",br"\input{a}")]);
    let a = ProjectAnalyzer::with_default_limits()
        .analyze(&source)
        .unwrap();
    assert!(
        a.dependency_graph()
            .edges()
            .iter()
            .any(|e| matches!(e.resolution(), DependencyResolution::Missing(_)))
    );
    assert!(
        a.dependency_graph()
            .edges()
            .iter()
            .any(|e| e.resolution() == &DependencyResolution::Dynamic)
    );
    for code in [
        DiagnosticCode::MissingProjectDependency,
        DiagnosticCode::DynamicDependency,
        DiagnosticCode::DependencyCycle,
        DiagnosticCode::DuplicateLabel,
        DiagnosticCode::UnresolvedReference,
    ] {
        assert!(
            a.diagnostics()
                .iter()
                .any(|d| d.diagnostic().code() == code),
            "missing {code:?}"
        );
    }
}
#[test]
fn source_validation() {
    assert!(ProjectSource::new(p("main.tex"), BTreeMap::new()).is_err());
    let mut files = BTreeMap::new();
    files.insert(p("other.tex"), Bytes::new());
    assert!(ProjectSource::new(p("main.tex"), files).is_err());
}
