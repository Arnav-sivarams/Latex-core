#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures use known-valid values"
)]

use blob_store::{
    BlobStore, BlobStoreError, BlobStoreMaintenance, DeleteOutcome, FsBlobStore, FsBlobStoreConfig,
    IntegrityMode, PutStatus,
};
use bytes::Bytes;
use core_types::BlobHash;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tempfile::TempDir;

fn config(mode: IntegrityMode) -> FsBlobStoreConfig {
    FsBlobStoreConfig::new(mode, 8).unwrap()
}
async fn store(root: &Path, mode: IntegrityMode) -> FsBlobStore {
    FsBlobStore::open(root, config(mode)).await.unwrap()
}
fn path(root: &Path, hash: BlobHash) -> PathBuf {
    let hex = hash.to_hex();
    root.join("sha256")
        .join(&hex[0..2])
        .join(&hex[2..4])
        .join(hex)
}

#[tokio::test]
async fn normal_zero_binary_identity_layout_and_metadata() {
    let root = TempDir::new().unwrap();
    let store = store(root.path(), IntegrityMode::VerifyOnRead).await;
    for data in [
        Bytes::from_static(b"\\documentclass{article}"),
        Bytes::new(),
        Bytes::from((0_u8..=255).collect::<Vec<_>>()),
    ] {
        let expected = BlobHash::digest(&data);
        let result = store.put(data.clone()).await.unwrap();
        assert_eq!(result.hash(), expected);
        assert_eq!(result.size_bytes(), data.len() as u64);
        assert_eq!(store.get(expected).await.unwrap(), data);
        assert!(store.exists(expected).await.unwrap());
        let metadata = store.metadata(expected).await.unwrap();
        assert_eq!(metadata.hash(), expected);
        assert_eq!(metadata.size_bytes(), data.len() as u64);
        assert!(path(root.path(), expected).is_file());
    }
    let missing = BlobHash::digest(b"missing");
    assert!(!store.exists(missing).await.unwrap());
    assert!(matches!(
        store.metadata(missing).await,
        Err(BlobStoreError::NotFound { .. })
    ));
}

#[tokio::test]
async fn sequential_dedup_verified_and_temp_cleanup() {
    let root = TempDir::new().unwrap();
    let store = store(root.path(), IntegrityMode::VerifyOnRead).await;
    let data = Bytes::from_static(b"deduplicate");
    let hash = BlobHash::digest(&data);
    assert_eq!(
        store
            .put_verified(hash, data.clone())
            .await
            .unwrap()
            .status(),
        PutStatus::Stored
    );
    assert_eq!(
        store.put(data.clone()).await.unwrap().status(),
        PutStatus::AlreadyPresent
    );
    let final_path = path(root.path(), hash);
    let shard = final_path.parent().unwrap();
    let entries: Vec<_> = fs::read_dir(shard).unwrap().collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(store.get(hash).await.unwrap(), data);
}

#[tokio::test]
async fn verified_mismatch_publishes_nothing() {
    let root = TempDir::new().unwrap();
    let store = store(root.path(), IntegrityMode::VerifyOnRead).await;
    let expected = BlobHash::digest(b"A");
    let actual = BlobHash::digest(b"B");
    assert!(
        matches!(store.put_verified(expected, Bytes::from_static(b"B")).await, Err(BlobStoreError::HashMismatch { expected: e, actual: a }) if e == expected && a == actual)
    );
    assert!(!store.exists(expected).await.unwrap());
    assert!(!store.exists(actual).await.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_identical_puts_converge() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(store(root.path(), IntegrityMode::VerifyOnRead).await);
    let data = Bytes::from(vec![19_u8; 16_384]);
    let expected = BlobHash::digest(&data);
    let mut tasks = Vec::new();
    for _ in 0..64 {
        let store = Arc::clone(&store);
        let data = data.clone();
        tasks.push(tokio::spawn(async move { store.put(data).await.unwrap() }));
    }
    let mut stored = 0;
    for task in tasks {
        let result = task.await.unwrap();
        assert_eq!(result.hash(), expected);
        if result.status() == PutStatus::Stored {
            stored += 1;
        }
    }
    assert!(stored >= 1);
    assert_eq!(store.get(expected).await.unwrap(), data);
    let final_path = path(root.path(), expected);
    let shard = final_path.parent().unwrap();
    assert_eq!(fs::read_dir(shard).unwrap().count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_distinct_puts_do_not_cross_content() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(store(root.path(), IntegrityMode::VerifyOnRead).await);
    let mut tasks = Vec::new();
    for index in 0_u8..64 {
        let store = Arc::clone(&store);
        tasks.push(tokio::spawn(async move {
            let bytes = Bytes::from(vec![index; 4096 + usize::from(index)]);
            let result = store.put(bytes.clone()).await.unwrap();
            (bytes, result.hash())
        }));
    }
    for task in tasks {
        let (bytes, hash) = task.await.unwrap();
        assert_eq!(hash, BlobHash::digest(&bytes));
        assert_eq!(store.get(hash).await.unwrap(), bytes);
    }
}

#[tokio::test]
async fn corruption_modes_and_duplicate_detection() {
    let root = TempDir::new().unwrap();
    let verifying = store(root.path(), IntegrityMode::VerifyOnRead).await;
    let original = Bytes::from_static(b"valid-data");
    let hash = verifying.put(original.clone()).await.unwrap().hash();
    let final_path = path(root.path(), hash);
    fs::write(&final_path, b"other-data").unwrap();
    assert!(matches!(
        verifying.get(hash).await,
        Err(BlobStoreError::CorruptBlob { .. })
    ));
    assert!(matches!(
        verifying.put(original.clone()).await,
        Err(BlobStoreError::CorruptBlob { .. })
    ));
    let trusting = store(root.path(), IntegrityMode::TrustStorage).await;
    assert_eq!(
        trusting.get(hash).await.unwrap(),
        Bytes::from_static(b"other-data")
    );
    assert!(matches!(
        trusting.put(original.clone()).await,
        Err(BlobStoreError::CorruptBlob { .. })
    ));
    fs::write(&final_path, b"tiny").unwrap();
    assert!(matches!(
        verifying.get(hash).await,
        Err(BlobStoreError::CorruptBlob { .. })
    ));
}

#[tokio::test]
async fn invalid_entry_is_rejected_by_all_operations() {
    let root = TempDir::new().unwrap();
    let store = store(root.path(), IntegrityMode::VerifyOnRead).await;
    let bytes = Bytes::from_static(b"blocked");
    let hash = BlobHash::digest(&bytes);
    let final_path = path(root.path(), hash);
    fs::create_dir_all(&final_path).unwrap();
    assert!(matches!(
        store.get(hash).await,
        Err(BlobStoreError::InvalidStorageEntry { .. })
    ));
    assert!(matches!(
        store.metadata(hash).await,
        Err(BlobStoreError::InvalidStorageEntry { .. })
    ));
    assert!(matches!(
        store.exists(hash).await,
        Err(BlobStoreError::InvalidStorageEntry { .. })
    ));
    assert!(matches!(
        store.put(bytes).await,
        Err(BlobStoreError::InvalidStorageEntry { .. })
    ));
}

#[tokio::test]
async fn reopen_trait_object_and_maintenance() {
    let root = TempDir::new().unwrap();
    let data = Bytes::from_static(b"persistent");
    let hash;
    {
        let concrete = store(root.path(), IntegrityMode::VerifyOnRead).await;
        let object: Arc<dyn BlobStore> = Arc::new(concrete);
        hash = object.put(data.clone()).await.unwrap().hash();
        assert_eq!(object.get(hash).await.unwrap(), data);
    }
    let reopened = store(root.path(), IntegrityMode::VerifyOnRead).await;
    assert_eq!(reopened.get(hash).await.unwrap(), data);
    assert_eq!(reopened.delete(hash).await.unwrap(), DeleteOutcome::Deleted);
    assert!(!reopened.exists(hash).await.unwrap());
    assert_eq!(
        reopened.delete(hash).await.unwrap(),
        DeleteOutcome::NotFound
    );
}

#[tokio::test]
async fn large_binary_round_trip() {
    let root = TempDir::new().unwrap();
    let store = store(root.path(), IntegrityMode::VerifyOnRead).await;
    let data = Bytes::from(
        (0_u8..=250)
            .cycle()
            .take(8 * 1024 * 1024)
            .collect::<Vec<_>>(),
    );
    let hash = store.put(data.clone()).await.unwrap().hash();
    assert_eq!(store.get(hash).await.unwrap(), data);
}

#[tokio::test]
async fn configuration_and_root_validation() {
    assert!(matches!(
        FsBlobStoreConfig::new(IntegrityMode::TrustStorage, 0),
        Err(BlobStoreError::InvalidConfiguration { .. })
    ));
    let default = FsBlobStoreConfig::development_default();
    assert_eq!(default.integrity_mode(), IntegrityMode::VerifyOnRead);
    assert_eq!(default.max_concurrent_io(), 32);
    let root = TempDir::new().unwrap();
    let file = root.path().join("file");
    fs::write(&file, b"x").unwrap();
    assert!(FsBlobStore::open(&file, default).await.is_err());
}
