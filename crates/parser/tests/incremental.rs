#![allow(clippy::unwrap_used, reason = "tests use known-valid fixtures")]
use bytes::Bytes;
use core_types::LogicalPath;
use latex_parser::*;
fn path() -> LogicalPath {
    LogicalPath::parse("main.tex").unwrap()
}
fn assert_fresh(session: &ParserSession) {
    let fresh = ParserSession::with_default_limits(path(), session.source().clone()).unwrap();
    assert_eq!(session.analysis(), fresh.analysis());
}
#[test]
fn incremental_title_edit_matches_fresh_parse() {
    let mut s = ParserSession::with_default_limits(
        path(),
        Bytes::from_static(b"\\section{Old}\n\\label{x} See \\ref{x}"),
    )
    .unwrap();
    s.apply_edit(TextEdit::new(9, 12, Bytes::from_static(b"New")))
        .unwrap();
    assert_eq!(s.stats().mode(), ParseMode::Incremental);
    assert_eq!(s.stats().generation(), 2);
    assert_eq!(s.analysis().sections()[0].title(), "New");
    assert_fresh(&s);
}
#[test]
fn sequential_unicode_newline_delete_malformed_repair_and_full_replace() {
    let mut s =
        ParserSession::with_default_limits(path(), Bytes::from_static(b"\\section{A}\nText"))
            .unwrap();
    for edit in [
        TextEdit::new(12, 12, Bytes::from_static(b"\n")),
        TextEdit::new(14, 14, Bytes::from("é")),
        TextEdit::new(16, 18, Bytes::new()),
        TextEdit::new(9, 10, Bytes::new()),
        TextEdit::new(9, 9, Bytes::from_static(b"A")),
    ] {
        s.apply_edit(edit).unwrap();
        assert_fresh(&s);
    }
    let before = s.stats().generation();
    s.replace_source(Bytes::from_static(b"\\chapter{Different}"))
        .unwrap();
    assert_eq!(s.stats().mode(), ParseMode::Full);
    assert_eq!(s.stats().generation(), before + 1);
    assert_eq!(s.analysis().sections()[0].title(), "Different");
    assert_fresh(&s);
}
#[test]
fn invalid_edits_are_atomic_and_points_use_byte_columns() {
    let mut s = ParserSession::with_default_limits(path(), Bytes::from("é\\section{A}")).unwrap();
    let source = s.source().clone();
    let analysis = s.analysis().clone();
    let stats = s.stats().clone();
    for edit in [
        TextEdit::new(4, 3, Bytes::new()),
        TextEdit::new(0, 999, Bytes::new()),
    ] {
        assert!(s.apply_edit(edit).is_err());
        assert_eq!(s.source(), &source);
        assert_eq!(s.analysis(), &analysis);
        assert_eq!(s.stats(), &stats);
    }
    s.apply_edit(TextEdit::new(2, 2, Bytes::from_static(b"\n")))
        .unwrap();
    assert_eq!(s.analysis().sections()[0].range().start().row(), 1);
    assert_eq!(s.analysis().sections()[0].range().start().column_bytes(), 0);
}
