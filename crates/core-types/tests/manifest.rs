#![allow(clippy::unwrap_used, reason = "test fixtures use known-valid values")]

use core_types::*;
use std::collections::BTreeMap;

fn entry(content: &[u8], size: u64) -> FileEntryV1 {
    FileEntryV1 {
        blob_hash: BlobHash::digest(content),
        size_bytes: size,
    }
}
fn manifest(reverse: bool) -> WorkspaceManifestV1 {
    let main = LogicalPath::parse("main.tex").unwrap();
    let other = LogicalPath::parse("தமிழ்/அறிக்கை.tex").unwrap();
    let pairs = if reverse {
        vec![
            (other, entry(b"other", 5)),
            (main.clone(), entry(b"main", 4)),
        ]
    } else {
        vec![
            (main.clone(), entry(b"main", 4)),
            (other, entry(b"other", 5)),
        ]
    };
    WorkspaceManifestV1::new(main, pairs.into_iter().collect()).unwrap()
}

#[test]
fn construction_and_determinism() {
    assert!(
        WorkspaceManifestV1::new(LogicalPath::parse("main.tex").unwrap(), BTreeMap::new()).is_err()
    );
    let mut missing = BTreeMap::new();
    missing.insert(LogicalPath::parse("other.tex").unwrap(), entry(b"x", 1));
    assert!(WorkspaceManifestV1::new(LogicalPath::parse("main.tex").unwrap(), missing).is_err());
    let a = manifest(false);
    let b = manifest(true);
    assert_eq!(
        a.canonical_json_bytes().unwrap(),
        b.canonical_json_bytes().unwrap()
    );
    assert_eq!(a.snapshot_id().unwrap(), b.snapshot_id().unwrap());
    assert_eq!(a.main_file().as_str(), "main.tex");
}

#[test]
fn every_manifest_input_changes_identity() {
    let base = manifest(false).snapshot_id().unwrap();
    let mut files = manifest(false).files().clone();
    files.insert(
        LogicalPath::parse("main.tex").unwrap(),
        entry(b"changed", 4),
    );
    assert_ne!(
        base,
        WorkspaceManifestV1::new(LogicalPath::parse("main.tex").unwrap(), files)
            .unwrap()
            .snapshot_id()
            .unwrap()
    );
    let files = manifest(false).files().clone();
    assert_ne!(
        base,
        WorkspaceManifestV1::new(LogicalPath::parse("தமிழ்/அறிக்கை.tex").unwrap(), files)
            .unwrap()
            .snapshot_id()
            .unwrap()
    );
    let mut files = manifest(false).files().clone();
    files
        .get_mut(&LogicalPath::parse("main.tex").unwrap())
        .unwrap()
        .size_bytes += 1;
    assert_ne!(
        base,
        WorkspaceManifestV1::new(LogicalPath::parse("main.tex").unwrap(), files)
            .unwrap()
            .snapshot_id()
            .unwrap()
    );
}

#[test]
fn serde_round_trip_preserves_canonical_bytes() {
    let original = manifest(false);
    let restored: WorkspaceManifestV1 =
        serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
    assert_eq!(original, restored);
    assert_eq!(
        original.canonical_json_bytes().unwrap(),
        restored.canonical_json_bytes().unwrap()
    );
}
