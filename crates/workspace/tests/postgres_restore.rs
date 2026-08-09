#![cfg(feature = "database-tests")]
#![allow(clippy::unwrap_used, reason = "integration fixtures fail fast")]

use blob_store::{BlobStore, BlobStoreMaintenance, FsBlobStore, FsBlobStoreConfig};
use bytes::Bytes;
use core_types::{BlobHash, LogicalPath, TenantId, UserId, WorkspaceId};
use persistence::{Database, DatabaseConfig, PostgresWorkspaceRepository};
use std::{collections::BTreeMap, env, fs, sync::Arc};
use tempfile::TempDir;
use workspace_model::{WorkspaceError, WorkspaceService};

fn path(value: &str) -> LogicalPath {
    LogicalPath::parse(value).unwrap()
}
async fn connect_database() -> Database {
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = Database::connect(DatabaseConfig::development(url).unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}
async fn service(
    root: &std::path::Path,
) -> (
    Database,
    PostgresWorkspaceRepository,
    Arc<FsBlobStore>,
    WorkspaceService,
    TenantId,
    UserId,
) {
    let database = connect_database().await;
    let repository = PostgresWorkspaceRepository::new(database.clone());
    let tenant = TenantId::new();
    let user = UserId::new();
    repository.create_test_identity(tenant, user).await.unwrap();
    let store = Arc::new(
        FsBlobStore::open(root, FsBlobStoreConfig::development_default())
            .await
            .unwrap(),
    );
    let blobs: Arc<dyn BlobStore> = store.clone();
    let service = WorkspaceService::new(repository.clone(), blobs);
    (database, repository, store, service, tenant, user)
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn process_restart_restores_snapshot_plus_tail_and_exact_binary_bytes() {
    let root = TempDir::new().unwrap();
    let (database, repository, store, service, tenant, user) = service(root.path()).await;
    let workspace = WorkspaceId::new();
    let main = path("main.tex");
    let mut version = service
        .create_workspace(
            tenant,
            user,
            workspace,
            main.clone(),
            Bytes::from_static(b"Version A"),
        )
        .await
        .unwrap()
        .version();
    let expected = [
        ("chapters/one.tex", b"Chapter one".as_slice()),
        ("references.bib", b"@book{x}".as_slice()),
        ("figures/raw.bin", &[0x00, 0xff, 0xc3, 0x28, 0x80][..]),
    ];
    for (name, bytes) in expected {
        version = service
            .put_file(
                workspace,
                user,
                version,
                path(name),
                Bytes::copy_from_slice(bytes),
            )
            .await
            .unwrap();
    }
    version = service
        .set_main_file(workspace, user, version, main.clone())
        .await
        .unwrap();
    let checkpoint = service.force_snapshot(workspace).await.unwrap();
    assert_eq!(checkpoint.workspace_version(), version);
    assert_eq!(
        checkpoint.snapshot_id().to_hex(),
        checkpoint.manifest_blob_hash().to_hex()
    );
    assert_eq!(service.force_snapshot(workspace).await.unwrap(), checkpoint);
    version = service
        .put_file(
            workspace,
            user,
            version,
            main.clone(),
            Bytes::from_static(b"Version B"),
        )
        .await
        .unwrap();
    version = service
        .put_file(
            workspace,
            user,
            version,
            path("after-snapshot.tex"),
            Bytes::from_static(b"tail event"),
        )
        .await
        .unwrap();
    assert!(checkpoint.workspace_version() < version);

    let expected: BTreeMap<_, _> = [
        (main.clone(), b"Version B".to_vec()),
        (path("chapters/one.tex"), b"Chapter one".to_vec()),
        (path("references.bib"), b"@book{x}".to_vec()),
        (path("figures/raw.bin"), vec![0x00, 0xff, 0xc3, 0x28, 0x80]),
        (path("after-snapshot.tex"), b"tail event".to_vec()),
    ]
    .into_iter()
    .collect();
    drop(service);
    drop(repository);
    drop(store);
    database.close().await;
    drop(database);

    let new_database = connect_database().await;
    let new_repository = PostgresWorkspaceRepository::new(new_database.clone());
    let new_store = Arc::new(
        FsBlobStore::open(root.path(), FsBlobStoreConfig::development_default())
            .await
            .unwrap(),
    );
    let new_blobs: Arc<dyn BlobStore> = new_store.clone();
    let new_service = WorkspaceService::new(new_repository, new_blobs);
    let restored = new_service.restore(workspace).await.unwrap();
    assert_eq!(restored.version(), version);
    assert_eq!(restored.main_file(), Some(&main));
    assert_eq!(restored.file_count(), expected.len());
    for (logical, bytes) in expected {
        let file = restored.file(&logical).unwrap();
        assert_eq!(file.blob_hash(), BlobHash::digest(&bytes));
        assert_eq!(file.size_bytes(), u64::try_from(bytes.len()).unwrap());
        assert_eq!(
            new_service
                .read_file(workspace, &logical)
                .await
                .unwrap()
                .as_ref(),
            bytes.as_slice()
        );
    }
    new_database.close().await;
}

#[tokio::test]
async fn stale_save_leaves_harmless_orphan_and_no_event() {
    let root = TempDir::new().unwrap();
    let (database, _, store, service, tenant, user) = service(root.path()).await;
    let workspace = WorkspaceId::new();
    let main = path("main.tex");
    let stale = service
        .create_workspace(
            tenant,
            user,
            workspace,
            main.clone(),
            Bytes::from_static(b"A"),
        )
        .await
        .unwrap()
        .version();
    let current = service
        .put_file(
            workspace,
            user,
            stale,
            main.clone(),
            Bytes::from_static(b"B"),
        )
        .await
        .unwrap();
    let orphan = Bytes::from_static(b"unique orphan C");
    let orphan_hash = BlobHash::digest(&orphan);
    assert!(matches!(
        service
            .put_file(workspace, user, stale, path("stale.tex"), orphan)
            .await,
        Err(WorkspaceError::VersionConflict { .. })
    ));
    assert!(store.exists(orphan_hash).await.unwrap());
    let restored = service.restore(workspace).await.unwrap();
    assert_eq!(restored.version(), current);
    assert!(
        !restored
            .files()
            .values()
            .any(|file| file.blob_hash() == orphan_hash)
    );
    database.close().await;
}

#[tokio::test]
async fn reverted_content_reuses_snapshot_identity_at_new_version() {
    let root = TempDir::new().unwrap();
    let (database, repository, _, service, tenant, user) = service(root.path()).await;
    let workspace = WorkspaceId::new();
    let main = path("main.tex");
    let mut version = service
        .create_workspace(
            tenant,
            user,
            workspace,
            main.clone(),
            Bytes::from_static(b"A"),
        )
        .await
        .unwrap()
        .version();
    let first = service.force_snapshot(workspace).await.unwrap();
    version = service
        .put_file(
            workspace,
            user,
            version,
            main.clone(),
            Bytes::from_static(b"B"),
        )
        .await
        .unwrap();
    version = service
        .put_file(workspace, user, version, main, Bytes::from_static(b"A"))
        .await
        .unwrap();
    let second = service.force_snapshot(workspace).await.unwrap();
    assert_eq!(first.snapshot_id(), second.snapshot_id());
    assert_eq!(first.manifest_blob_hash(), second.manifest_blob_hash());
    assert_ne!(first.workspace_version(), second.workspace_version());
    assert_eq!(second.workspace_version(), version);
    assert_eq!(
        repository
            .load_head(workspace)
            .await
            .unwrap()
            .latest_snapshot()
            .unwrap()
            .workspace_version(),
        version
    );
    database.close().await;
}

#[tokio::test]
async fn missing_and_corrupt_latest_manifests_are_not_ignored() {
    for corrupt in [false, true] {
        let root = TempDir::new().unwrap();
        let (database, _, store, service, tenant, user) = service(root.path()).await;
        let workspace = WorkspaceId::new();
        service
            .create_workspace(
                tenant,
                user,
                workspace,
                path("main.tex"),
                Bytes::from_static(b"A"),
            )
            .await
            .unwrap();
        let checkpoint = service.force_snapshot(workspace).await.unwrap();
        if corrupt {
            let hex = checkpoint.manifest_blob_hash().to_hex();
            let file = root
                .path()
                .join("sha256")
                .join(&hex[0..2])
                .join(&hex[2..4])
                .join(hex);
            fs::write(file, b"corrupt").unwrap();
        } else {
            store.delete(checkpoint.manifest_blob_hash()).await.unwrap();
        }
        assert!(service.restore(workspace).await.is_err());
        database.close().await;
    }
}
