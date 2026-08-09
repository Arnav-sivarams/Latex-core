#![allow(clippy::unwrap_used, reason = "test fixtures use known-valid values")]

use core_types::LogicalPath;
use proptest::prelude::*;

#[test]
fn listed_examples_validate() {
    for value in [
        "main.tex",
        "chapter 1/introduction.tex",
        "figures/plot@2.pdf",
        "styles/my-style.sty",
        "தமிழ்/அறிக்கை.tex",
        "report..draft.tex",
    ] {
        assert!(LogicalPath::parse(value).is_ok(), "{value}");
    }
    for value in [
        "",
        "/main.tex",
        "../main.tex",
        "chapters/../main.tex",
        "./main.tex",
        "chapters//main.tex",
        "chapters/",
        "C:\\main.tex",
        "C:/main.tex",
        "folder\\main.tex",
        "bad\0name",
    ] {
        assert!(LogicalPath::parse(value).is_err(), "{value}");
    }
    let path = LogicalPath::parse("chapter 1/main.tex").unwrap();
    assert_eq!(path.file_name(), "main.tex");
    assert_eq!(path.extension(), Some("tex"));
}

proptest! {
    #[test] fn complete_parent_segment_rejected(a in "[a-z]{1,12}", b in "[a-z]{1,12}") { let rejected = LogicalPath::parse(&format!("{a}/../{b}")).is_err(); prop_assert!(rejected); }
    #[test] fn absolute_rejected(tail in "[a-z/]{1,30}") { let rejected = LogicalPath::parse(&format!("/{tail}")).is_err(); prop_assert!(rejected); }
    #[test] fn backslash_rejected(a in "[a-z]{1,12}", b in "[a-z]{1,12}") { let rejected = LogicalPath::parse(&format!("{a}\\{b}")).is_err(); prop_assert!(rejected); }
    #[test] fn accepted_paths_serde_round_trip(parts in prop::collection::vec("[a-zA-Z0-9@._ -]{1,20}", 1..5)) {
        let value = parts.join("/");
        if let Ok(path) = LogicalPath::parse(&value) { prop_assert_eq!(serde_json::from_str::<LogicalPath>(&serde_json::to_string(&path).unwrap()).unwrap(), path); }
    }
}
