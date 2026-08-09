#![allow(clippy::unwrap_used, reason = "tests use known-valid fixtures")]
use bytes::Bytes;
use core_types::LogicalPath;
use latex_parser::*;
fn parse(bytes: Bytes) -> ParserSession {
    ParserSession::with_default_limits(LogicalPath::parse("main.tex").unwrap(), bytes).unwrap()
}
#[test]
fn incomplete_documents_return_partial_analysis() {
    for source in [
        b"\\section{unfinished".as_slice(),
        b"\\begin{itemize}".as_slice(),
        b"\\includegraphics{".as_slice(),
        b"random } brace".as_slice(),
        b"\\documentclass{article}\\begin{document}".as_slice(),
    ] {
        let session = parse(Bytes::copy_from_slice(source));
        assert!(!session.analysis().diagnostics().is_empty());
    }
}
#[test]
fn invalid_utf8_is_preserved_and_diagnosed() {
    let bytes = Bytes::from_static(b"\\section{bad\xfftext}");
    let session = parse(bytes.clone());
    assert_eq!(session.source(), &bytes);
    assert!(
        session
            .analysis()
            .diagnostics()
            .iter()
            .any(|d| d.code() == DiagnosticCode::InvalidUtf8SemanticText)
    );
}
#[test]
fn semantic_limits_bound_collection() {
    let limits = ParserLimits::new(3, 32).unwrap();
    let source =
        Bytes::from_static(b"\\section{a}\\section{b}\\section{c}\\section{d}\\section{e}");
    let session = ParserSession::new(
        LogicalPath::parse("main.tex").unwrap(),
        source.clone(),
        limits,
    )
    .unwrap();
    assert_eq!(session.source(), &source);
    assert!(
        session
            .analysis()
            .diagnostics()
            .iter()
            .any(|d| d.code() == DiagnosticCode::ExtractionLimitReached)
    );
    assert!(session.analysis().sections().len() <= 3);
}
#[test]
fn two_mebibyte_source_is_stable() {
    let mut source = Vec::with_capacity(2 * 1024 * 1024 + 32);
    source.extend_from_slice(b"\\documentclass{article}\n");
    source.resize(2 * 1024 * 1024 + 32, b'a');
    let session = parse(Bytes::from(source));
    assert_eq!(session.stats().source_bytes(), 2 * 1024 * 1024 + 32);
}
