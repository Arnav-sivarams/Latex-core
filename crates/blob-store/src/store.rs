//! Portable asynchronous blob storage contracts.

use crate::{BlobMetadata, BlobStoreError, DeleteOutcome, PutResult};
use async_trait::async_trait;
use bytes::Bytes;
use core_types::BlobHash;

/// Immutable content-addressed storage whose hashes identify original bytes.
///
/// Implementations must be safe under concurrent calls and must not interpret content.
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, bytes: Bytes) -> Result<PutResult, BlobStoreError>;
    async fn put_verified(
        &self,
        expected: BlobHash,
        bytes: Bytes,
    ) -> Result<PutResult, BlobStoreError>;
    async fn get(&self, hash: BlobHash) -> Result<Bytes, BlobStoreError>;
    async fn exists(&self, hash: BlobHash) -> Result<bool, BlobStoreError>;
    async fn metadata(&self, hash: BlobHash) -> Result<BlobMetadata, BlobStoreError>;
}

/// Destructive operations reserved for controlled maintenance paths.
#[async_trait]
pub trait BlobStoreMaintenance: Send + Sync {
    async fn delete(&self, hash: BlobHash) -> Result<DeleteOutcome, BlobStoreError>;
}
