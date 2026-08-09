#![allow(clippy::unwrap_used, reason = "test fixtures use known-valid values")]

use core_types::*;

fn key() -> CompileKey {
    BlobHash::digest(b"key").to_string().parse().unwrap()
}
fn artifact(kind: ArtifactKind, name: &str, size: u64) -> ArtifactRefV1 {
    ArtifactRefV1 {
        artifact_id: ArtifactId::new(),
        kind,
        logical_name: LogicalPath::parse(name).unwrap(),
        blob_hash: BlobHash::digest(name.as_bytes()),
        size_bytes: size,
    }
}

#[test]
fn ordering_duplicates_and_zero_bytes() {
    let manifest = ArtifactManifestV1::new(
        key(),
        vec![
            artifact(ArtifactKind::Log, "z.log", 0),
            artifact(ArtifactKind::Pdf, "output.pdf", 4),
            artifact(ArtifactKind::Log, "a.log", 2),
        ],
    )
    .unwrap();
    assert_eq!(manifest.artifacts()[0].kind, ArtifactKind::Pdf);
    assert_eq!(manifest.artifacts()[1].logical_name.as_str(), "a.log");
    assert_eq!(manifest.artifacts()[2].size_bytes, 0);
    let duplicate = vec![
        artifact(ArtifactKind::Log, "same.log", 1),
        artifact(ArtifactKind::Log, "same.log", 2),
    ];
    assert!(ArtifactManifestV1::new(key(), duplicate).is_err());
}

#[test]
fn serde_round_trip_and_unicode_name() {
    let manifest = ArtifactManifestV1::new(
        key(),
        vec![artifact(ArtifactKind::Other, "முடிவு/அறிக்கை.txt", 3)],
    )
    .unwrap();
    let restored: ArtifactManifestV1 =
        serde_json::from_str(&serde_json::to_string(&manifest).unwrap()).unwrap();
    assert_eq!(manifest, restored);
    assert_eq!(
        manifest.canonical_json_bytes().unwrap(),
        restored.canonical_json_bytes().unwrap()
    );
}
