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

#[test]
fn parses_tex_live_special_metadata_prefix() {
    let db = "name 00texlive.config\ncategory TLCore\ndepend minrelease/2016\ndepend release/2026\n\nname 00texlive.installation\ncategory TLCore\ndepend opt_create_formats:1\ndepend setting_available_architectures:x86_64-linux\n\nname article\ncategory Package\nrevision 123\nrunfiles\n tex/latex/base/article.cls";
    let p = parse_tlpdb(db).unwrap();

    assert_eq!(p.len(), 1);
    assert!(!p.contains_key("00texlive.config"));
    assert!(!p.contains_key("00texlive.installation"));
    assert_eq!(p["article"].revision(), 123);
}

#[test]
fn rejects_revisionless_non_metadata_records() {
    for db in [
        "name ordinary\ncategory Package",
        "name ordinary\ncategory Collection",
        "name arbitrary-meta\ncategory TLCore",
    ] {
        assert!(parse_tlpdb(db).is_err(), "{db}");
    }
}

#[test]
fn rejects_revisionless_special_metadata_with_wrong_category() {
    for name in ["00texlive.config", "00texlive.installation"] {
        let db = format!("name {name}\ncategory Package");
        assert!(parse_tlpdb(&db).is_err(), "{db}");
    }
}

#[test]
fn rejects_duplicate_special_metadata_records() {
    let db = "name 00texlive.config\ncategory TLCore\n\nname 00texlive.config\ncategory TLCore";
    assert!(parse_tlpdb(db).is_err());
}
