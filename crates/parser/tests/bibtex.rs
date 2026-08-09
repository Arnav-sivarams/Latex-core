#![allow(clippy::unwrap_used, reason = "tests use controlled fixtures")]
use bytes::Bytes;
use core_types::LogicalPath;
use latex_parser::*;

fn parse(bytes: Bytes) -> BibtexSession {
    BibtexSession::new(LogicalPath::parse("refs.bib").unwrap(), bytes).unwrap()
}
#[test]
fn entries_fields_strings_unicode_and_determinism() {
    let source = Bytes::from(
        "@string{j = {Journal}}\n@article{smith2024, author={Åda}, title=\"{Title}\", year=2024}\n@book(other, doi={x})\n@custom{x, value=j}",
    );
    let a = parse(source.clone());
    let b = parse(source);
    assert_eq!(a.analysis(), b.analysis());
    assert_eq!(a.analysis().entries().len(), 3);
    assert_eq!(a.analysis().entries()[0].entry_type(), "article");
    assert_eq!(a.analysis().entries()[0].key(), "smith2024");
    assert_eq!(a.analysis().entries()[0].fields()[0].raw_value(), "{Åda}");
    assert_eq!(a.analysis().string_definitions()[0].name(), "j");
}
#[test]
fn malformed_and_invalid_utf8_are_safe() {
    let malformed = parse(Bytes::from_static(b"@article{good,title={x}}\n@book{half,"));
    assert!(
        malformed
            .analysis()
            .entries()
            .iter()
            .any(|e| e.key() == "good")
    );
    assert!(
        malformed
            .analysis()
            .diagnostics()
            .iter()
            .any(|d| d.code() == DiagnosticCode::BibtexSyntaxError)
    );
    let invalid = parse(Bytes::from_static(b"@article{bad,title={\xff}}"));
    assert!(
        invalid
            .analysis()
            .diagnostics()
            .iter()
            .any(|d| d.code() == DiagnosticCode::InvalidUtf8SemanticText)
    );
}
#[test]
fn duplicate_entries_remain_extractable() {
    let a = parse(Bytes::from_static(b"@article{x}\n@book{x}"));
    assert_eq!(
        a.analysis()
            .entries()
            .iter()
            .filter(|e| e.key() == "x")
            .count(),
        2
    );
}
