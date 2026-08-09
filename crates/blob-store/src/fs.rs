//! Crash-safe local filesystem blob storage.

use crate::{
    BlobMetadata, BlobStore, BlobStoreError, BlobStoreMaintenance, DeleteOutcome,
    FsBlobStoreConfig, IntegrityMode, PutResult, PutStatus,
};
use async_trait::async_trait;
use bytes::Bytes;
use core_types::BlobHash;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tempfile::Builder;
use tokio::sync::Semaphore;

/// Filesystem storage using `sha256/HH/HH/FULL_HASH` paths.
///
/// Writes are synced before no-clobber publication, so final paths expose complete bytes.
/// The root must be service-owned and unavailable to untrusted LaTeX. Blocking I/O is
/// bounded per instance. Crashes can leave `.tmp-blob-*` files; they are never valid blobs
/// and are not removed on open because another process may still be writing them.
#[derive(Clone, Debug)]
pub struct FsBlobStore {
    root: Arc<PathBuf>,
    config: FsBlobStoreConfig,
    semaphore: Arc<Semaphore>,
}

impl FsBlobStore {
    /// Opens or creates a filesystem store.
    ///
    /// # Errors
    /// Returns an error for invalid roots, filesystem failures, or failed blocking tasks.
    pub async fn open(
        root: impl AsRef<Path>,
        config: FsBlobStoreConfig,
    ) -> Result<Self, BlobStoreError> {
        let root = root.as_ref().to_path_buf();
        if root.as_os_str().is_empty() {
            return Err(BlobStoreError::InvalidConfiguration {
                message: "storage root must not be empty".to_owned(),
            });
        }
        let permits = config.max_concurrent_io();
        let semaphore = Arc::new(Semaphore::new(permits));
        let store = Self {
            root: Arc::new(root),
            config,
            semaphore,
        };
        let root = Arc::clone(&store.root);
        store
            .blocking(move || {
                fs::create_dir_all(root.join("sha256")).map_err(|source| {
                    io_error("create store directories", root.as_ref().clone(), source)
                })?;
                require_directory(root.as_ref())?;
                require_directory(&root.join("sha256"))
            })
            .await?;
        Ok(store)
    }

    fn path(&self, hash: BlobHash) -> PathBuf {
        let hex = hash.to_hex();
        self.root
            .join("sha256")
            .join(&hex[0..2])
            .join(&hex[2..4])
            .join(hex)
    }

    async fn blocking<T, F>(&self, operation: F) -> Result<T, BlobStoreError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, BlobStoreError> + Send + 'static,
    {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| BlobStoreError::InvalidConfiguration {
                message: "filesystem I/O semaphore closed".to_owned(),
            })?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        })
        .await
        .map_err(|source| BlobStoreError::BlockingTaskFailed { source })?
    }

    async fn write_verified(
        &self,
        expected: BlobHash,
        bytes: Bytes,
    ) -> Result<PutResult, BlobStoreError> {
        let actual = BlobHash::digest(&bytes);
        if actual != expected {
            return Err(BlobStoreError::HashMismatch { expected, actual });
        }
        let size =
            u64::try_from(bytes.len()).map_err(|_| BlobStoreError::InvalidConfiguration {
                message: "blob length does not fit u64".to_owned(),
            })?;
        let final_path = self.path(expected);
        self.blocking(move || publish(final_path, expected, &bytes, size))
            .await
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn put(&self, bytes: Bytes) -> Result<PutResult, BlobStoreError> {
        self.write_verified(BlobHash::digest(&bytes), bytes).await
    }
    async fn put_verified(
        &self,
        expected: BlobHash,
        bytes: Bytes,
    ) -> Result<PutResult, BlobStoreError> {
        self.write_verified(expected, bytes).await
    }
    async fn get(&self, hash: BlobHash) -> Result<Bytes, BlobStoreError> {
        let path = self.path(hash);
        let verify = self.config.integrity_mode() == IntegrityMode::VerifyOnRead;
        self.blocking(move || {
            let metadata = regular_metadata(&path, hash)?;
            let data =
                fs::read(&path).map_err(|source| io_error("read blob", path.clone(), source))?;
            if metadata.len() != data.len() as u64 {
                return Err(corrupt_size(hash, &data));
            }
            if verify {
                let actual = BlobHash::digest(&data);
                if actual != hash {
                    return Err(BlobStoreError::CorruptBlob {
                        expected: hash,
                        actual,
                    });
                }
            }
            Ok(Bytes::from(data))
        })
        .await
    }
    async fn exists(&self, hash: BlobHash) -> Result<bool, BlobStoreError> {
        let path = self.path(hash);
        self.blocking(move || match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => Ok(true),
            Ok(_) => Err(invalid_entry(path)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(io_error("inspect blob", path, source)),
        })
        .await
    }
    async fn metadata(&self, hash: BlobHash) -> Result<BlobMetadata, BlobStoreError> {
        let path = self.path(hash);
        self.blocking(move || {
            Ok(BlobMetadata::new(
                hash,
                regular_metadata(&path, hash)?.len(),
            ))
        })
        .await
    }
}

#[async_trait]
impl BlobStoreMaintenance for FsBlobStore {
    async fn delete(&self, hash: BlobHash) -> Result<DeleteOutcome, BlobStoreError> {
        let path = self.path(hash);
        self.blocking(move || match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::remove_file(&path).map_err(|source| io_error("delete blob", path, source))?;
                Ok(DeleteOutcome::Deleted)
            }
            Ok(_) => Err(invalid_entry(path)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(DeleteOutcome::NotFound),
            Err(source) => Err(io_error("inspect blob for deletion", path, source)),
        })
        .await
    }
}

fn publish(
    path: PathBuf,
    hash: BlobHash,
    bytes: &[u8],
    size: u64,
) -> Result<PutResult, BlobStoreError> {
    if path_exists(&path)? {
        verify_existing(&path, hash, size)?;
        return Ok(PutResult::new(hash, size, PutStatus::AlreadyPresent));
    }
    let parent = path
        .parent()
        .ok_or_else(|| BlobStoreError::InvalidConfiguration {
            message: "final blob path has no parent".to_owned(),
        })?
        .to_path_buf();
    fs::create_dir_all(&parent)
        .map_err(|source| io_error("create shard directories", parent.clone(), source))?;
    let mut temporary = Builder::new()
        .prefix(".tmp-blob-")
        .tempfile_in(&parent)
        .map_err(|source| io_error("create temporary blob", parent.clone(), source))?;
    temporary.write_all(bytes).map_err(|source| {
        io_error(
            "write temporary blob",
            temporary.path().to_path_buf(),
            source,
        )
    })?;
    temporary.flush().map_err(|source| {
        io_error(
            "flush temporary blob",
            temporary.path().to_path_buf(),
            source,
        )
    })?;
    temporary.as_file().sync_all().map_err(|source| {
        io_error(
            "sync temporary blob",
            temporary.path().to_path_buf(),
            source,
        )
    })?;
    if temporary
        .as_file()
        .metadata()
        .map_err(|source| {
            io_error(
                "inspect temporary blob",
                temporary.path().to_path_buf(),
                source,
            )
        })?
        .len()
        != size
    {
        return Err(corrupt_size(hash, bytes));
    }
    match temporary.persist_noclobber(&path) {
        Ok(_) => {
            sync_directory(&parent)?;
            Ok(PutResult::new(hash, size, PutStatus::Stored))
        }
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            drop(error.file);
            verify_existing(&path, hash, size)?;
            Ok(PutResult::new(hash, size, PutStatus::AlreadyPresent))
        }
        Err(error) => Err(io_error("publish blob", path, error.error)),
    }
}

fn path_exists(path: &Path) -> Result<bool, BlobStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(invalid_entry(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error("inspect blob", path.to_path_buf(), source)),
    }
}
fn verify_existing(path: &Path, expected: BlobHash, size: u64) -> Result<(), BlobStoreError> {
    let metadata = regular_metadata(path, expected)?;
    let data = fs::read(path)
        .map_err(|source| io_error("read existing blob", path.to_path_buf(), source))?;
    let actual = BlobHash::digest(&data);
    if actual != expected || metadata.len() != size || data.len() as u64 != size {
        return Err(BlobStoreError::CorruptBlob { expected, actual });
    }
    Ok(())
}
fn regular_metadata(path: &Path, hash: BlobHash) -> Result<fs::Metadata, BlobStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(metadata),
        Ok(_) => Err(invalid_entry(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(BlobStoreError::NotFound { hash })
        }
        Err(source) => Err(io_error("inspect blob", path.to_path_buf(), source)),
    }
}
fn require_directory(path: &Path) -> Result<(), BlobStoreError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect store directory", path.to_path_buf(), source))?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(invalid_entry(path.to_path_buf()))
    }
}
fn sync_directory(path: &Path) -> Result<(), BlobStoreError> {
    let directory = fs::File::open(path)
        .map_err(|source| io_error("open shard directory for sync", path.to_path_buf(), source))?;
    directory
        .sync_all()
        .map_err(|source| io_error("sync shard directory", path.to_path_buf(), source))
}
fn corrupt_size(expected: BlobHash, bytes: &[u8]) -> BlobStoreError {
    BlobStoreError::CorruptBlob {
        expected,
        actual: BlobHash::digest(bytes),
    }
}
fn invalid_entry(path: PathBuf) -> BlobStoreError {
    BlobStoreError::InvalidStorageEntry {
        path,
        kind: "expected a regular file or directory",
    }
}
fn io_error(operation: &'static str, path: PathBuf, source: io::Error) -> BlobStoreError {
    BlobStoreError::Io {
        operation,
        path,
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn blocking_helper_enforces_configured_limit() {
        let directory = tempfile::tempdir().expect("temporary test directory");
        let store = FsBlobStore::open(
            directory.path(),
            FsBlobStoreConfig::new(IntegrityMode::VerifyOnRead, 2).expect("valid config"),
        )
        .await
        .expect("open store");
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let store = store.clone();
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            tasks.push(tokio::spawn(async move {
                store
                    .blocking(move || {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        maximum.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(5));
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .await
            }));
        }
        for task in tasks {
            task.await
                .expect("join operation")
                .expect("operation succeeds");
        }
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
    }
}
