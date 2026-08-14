#![allow(clippy::unwrap_used, reason = "tests use known-valid fixtures")]

use core_types::{BlobHash, LogicalPath, WorkspaceId, WorkspaceVersion};
use persistence::WorkspaceEventRecord;
use serde_json::json;
use workspace_model::{
    WorkspaceError, WorkspaceMutationV1, WorkspaceOperationV1, WorkspaceService, WorkspaceState,
};

fn path(value: &str) -> LogicalPath {
    LogicalPath::parse(value).unwrap()
}
fn put(name: &str, bytes: &[u8]) -> WorkspaceOperationV1 {
    WorkspaceOperationV1::PutFile {
        path: path(name),
        blob_hash: BlobHash::digest(bytes),
        size_bytes: u64::try_from(bytes.len()).unwrap(),
    }
}
fn initial() -> WorkspaceState {
    let mut state = WorkspaceState::empty(WorkspaceId::new());
    state
        .apply(
            &WorkspaceMutationV1::new(vec![
                put("main.tex", b"A"),
                WorkspaceOperationV1::SetMainFile {
                    path: path("main.tex"),
                },
            ])
            .unwrap(),
            WorkspaceVersion::initial(),
        )
        .unwrap();
    state
}

#[test]
fn event_round_trip_all_variants_unicode_and_schema_rejection() {
    let mutation = WorkspaceMutationV1::new(vec![
        put("தமிழ்/அறிக்கை.tex", b"x"),
        WorkspaceOperationV1::RenameFile {
            from: path("தமிழ்/அறிக்கை.tex"),
            to: path("paper.tex"),
        },
        WorkspaceOperationV1::SetMainFile {
            path: path("paper.tex"),
        },
        WorkspaceOperationV1::DeleteFile {
            path: path("old.tex"),
        },
    ])
    .unwrap();
    let value = serde_json::to_value(&mutation).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(
        serde_json::from_value::<WorkspaceMutationV1>(value.clone()).unwrap(),
        mutation
    );
    for version in [0, 2, 999] {
        let mut invalid = value.clone();
        invalid["schema_version"] = json!(version);
        assert!(serde_json::from_value::<WorkspaceMutationV1>(invalid).is_err());
    }
    let mut missing = value;
    missing.as_object_mut().unwrap().remove("schema_version");
    assert!(serde_json::from_value::<WorkspaceMutationV1>(missing).is_err());
    assert!(matches!(
        WorkspaceMutationV1::new(Vec::new()),
        Err(WorkspaceError::EmptyMutation)
    ));
    assert!(
        serde_json::from_value::<WorkspaceMutationV1>(json!({"schema_version":1,"operations":[]}))
            .is_err()
    );
}

#[test]
fn mutation_is_atomic_and_version_conflicts_do_not_change_state() {
    let mut state = initial();
    let original = state.clone();
    let mutation = WorkspaceMutationV1::new(vec![
        put("second.tex", b"B"),
        WorkspaceOperationV1::SetMainFile {
            path: path("second.tex"),
        },
        WorkspaceOperationV1::DeleteFile {
            path: path("missing.tex"),
        },
    ])
    .unwrap();
    assert!(matches!(
        state.apply(&mutation, WorkspaceVersion::new(1)),
        Err(WorkspaceError::FileNotFound { .. })
    ));
    assert_eq!(state, original);
    assert!(matches!(
        state.apply(
            &WorkspaceMutationV1::new(vec![put("x.tex", b"x")]).unwrap(),
            WorkspaceVersion::initial()
        ),
        Err(WorkspaceError::VersionConflict { .. })
    ));
    assert_eq!(state, original);
}

#[test]
fn rename_and_main_file_rules_are_exact() {
    let mut state = initial();
    let before = state.file(&path("main.tex")).unwrap().clone();
    state
        .apply(
            &WorkspaceMutationV1::new(vec![WorkspaceOperationV1::RenameFile {
                from: path("main.tex"),
                to: path("paper.tex"),
            }])
            .unwrap(),
            WorkspaceVersion::new(1),
        )
        .unwrap();
    assert!(!state.contains_file(&path("main.tex")));
    assert_eq!(state.file(&path("paper.tex")), Some(&before));
    assert_eq!(state.main_file(), Some(&path("paper.tex")));
    state
        .apply(
            &WorkspaceMutationV1::new(vec![WorkspaceOperationV1::RenameFile {
                from: path("paper.tex"),
                to: path("paper.tex"),
            }])
            .unwrap(),
            WorkspaceVersion::new(2),
        )
        .unwrap();
    state
        .apply(
            &WorkspaceMutationV1::new(vec![
                put("second.tex", b"B"),
                WorkspaceOperationV1::SetMainFile {
                    path: path("second.tex"),
                },
                WorkspaceOperationV1::DeleteFile {
                    path: path("paper.tex"),
                },
            ])
            .unwrap(),
            WorkspaceVersion::new(3),
        )
        .unwrap();
    assert_eq!(state.main_file(), Some(&path("second.tex")));
}

#[test]
fn deleting_main_file_clears_main_and_preserves_other_file_blobs() {
    let mut state = initial();
    let original = state.file(&path("main.tex")).unwrap().clone();
    state
        .apply(
            &WorkspaceMutationV1::new(vec![
                put("sections/intro.tex", b"intro"),
                WorkspaceOperationV1::DeleteFile {
                    path: path("main.tex"),
                },
            ])
            .unwrap(),
            WorkspaceVersion::new(1),
        )
        .unwrap();
    assert_eq!(state.main_file(), None);
    assert!(!state.contains_file(&path("main.tex")));
    assert_eq!(
        state
            .file(&path("sections/intro.tex"))
            .unwrap()
            .size_bytes(),
        5
    );
    assert_eq!(original.size_bytes(), 1);
    assert!(state.to_manifest().is_err());
}

#[test]
fn nested_rename_is_atomic_reuses_the_blob_and_rejects_conflicts() {
    let mut state = initial();
    state
        .apply(
            &WorkspaceMutationV1::new(vec![put("sections/intro.tex", b"intro")]).unwrap(),
            WorkspaceVersion::new(1),
        )
        .unwrap();
    let original = state.file(&path("sections/intro.tex")).unwrap().clone();
    state
        .apply(
            &WorkspaceMutationV1::new(vec![WorkspaceOperationV1::RenameFile {
                from: path("sections/intro.tex"),
                to: path("chapters/introduction.tex"),
            }])
            .unwrap(),
            WorkspaceVersion::new(2),
        )
        .unwrap();
    assert_eq!(state.version(), WorkspaceVersion::new(3));
    assert!(!state.contains_file(&path("sections/intro.tex")));
    assert_eq!(
        state.file(&path("chapters/introduction.tex")),
        Some(&original)
    );
    let before_conflict = state.clone();
    assert!(matches!(
        state.apply(
            &WorkspaceMutationV1::new(vec![WorkspaceOperationV1::RenameFile {
                from: path("chapters/introduction.tex"),
                to: path("main.tex"),
            }])
            .unwrap(),
            WorkspaceVersion::new(3),
        ),
        Err(WorkspaceError::FileAlreadyExists { .. })
    ));
    assert_eq!(state, before_conflict);
    assert!(matches!(
        state.apply(
            &WorkspaceMutationV1::new(vec![WorkspaceOperationV1::RenameFile {
                from: path("chapters/introduction.tex"),
                to: path("appendix/introduction.tex"),
            }])
            .unwrap(),
            WorkspaceVersion::new(2),
        ),
        Err(WorkspaceError::VersionConflict { .. })
    ));
}

#[test]
fn manifest_round_trip_and_v1_hash_relationship() {
    let state = initial();
    let manifest = state.to_manifest().unwrap();
    let bytes = manifest.canonical_json_bytes().unwrap();
    assert_eq!(
        manifest.snapshot_id().unwrap().to_hex(),
        BlobHash::digest(&bytes).to_hex()
    );
    assert_eq!(
        WorkspaceState::from_manifest(state.workspace_id(), state.version(), &manifest).unwrap(),
        state
    );
    assert!(
        WorkspaceState::empty(WorkspaceId::new())
            .to_manifest()
            .is_err()
    );
}

#[test]
fn persistent_replay_rejects_corruption() {
    let payload =
        serde_json::to_value(WorkspaceMutationV1::new(vec![put("x.tex", b"x")]).unwrap()).unwrap();
    let cases = [
        WorkspaceEventRecord::new(
            WorkspaceVersion::new(3),
            WorkspaceVersion::new(1),
            "workspace.mutation".into(),
            1,
            payload.clone(),
        ),
        WorkspaceEventRecord::new(
            WorkspaceVersion::new(2),
            WorkspaceVersion::new(0),
            "workspace.mutation".into(),
            1,
            payload.clone(),
        ),
        WorkspaceEventRecord::new(
            WorkspaceVersion::new(2),
            WorkspaceVersion::new(1),
            "unknown".into(),
            1,
            payload.clone(),
        ),
        WorkspaceEventRecord::new(
            WorkspaceVersion::new(2),
            WorkspaceVersion::new(1),
            "workspace.mutation".into(),
            2,
            payload,
        ),
        WorkspaceEventRecord::new(
            WorkspaceVersion::new(2),
            WorkspaceVersion::new(1),
            "workspace.mutation".into(),
            1,
            json!({"schema_version":999,"operations":[]}),
        ),
    ];
    for record in cases {
        let mut state = initial();
        let original = state.clone();
        assert!(WorkspaceService::replay_event(&mut state, &record).is_err());
        assert_eq!(state, original);
    }
}
