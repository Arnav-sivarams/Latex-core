#![allow(clippy::unwrap_used, reason = "controlled fixtures")]
use tex_index::*;
#[test]
fn parses_subset_crlf_unknown_and_final_record() {
    let db = "name alpha\r\ncategory Package\r\nrevision 12\r\ncatalogue-version 1.2\r\ncatalogue-license lppl\r\nunknown future\r\nrunfiles size=1\r\n tex/latex/a/a.sty\r\nbinfiles arch=1\r\n bin/x/tool\r\n\r\nname beta\r\ncategory Collection\r\nrevision 3";
    let p = parse_tlpdb(db).unwrap();
    assert_eq!(p.len(), 2);
    let a = &p["alpha"];
    assert_eq!(a.revision(), 12);
    assert_eq!(a.catalogue_version(), Some("1.2"));
    assert_eq!(a.runfiles(), ["tex/latex/a/a.sty"]);
    assert_eq!(a.binfiles(), ["bin/x/tool"]);
}
#[test]
fn rejects_malformed_records_and_paths() {
    for db in [
        "category Package\nrevision 1",
        "name x\ncategory Package\nrevision nope",
        "name x\ncategory Package\nrevision 1\nrunfiles\n /absolute",
        "name x\ncategory Package\nrevision 1\nrunfiles\n tex/../evil",
        "name x\ncategory Package\nrevision 1\n\nname x\ncategory Package\nrevision 2",
    ] {
        assert!(parse_tlpdb(db).is_err(), "{db}");
    }
}
