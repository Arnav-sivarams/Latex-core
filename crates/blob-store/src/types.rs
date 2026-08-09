//! Public blob storage value types and filesystem configuration.

use crate::BlobStoreError;
use core_types::BlobHash;

/// Outcome of an immutable put operation.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PutStatus {
    /// This operation published the object.
    Stored,
    /// An identical valid object was already published.
    AlreadyPresent,
}

/// Result of storing immutable bytes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PutResult {
    hash: BlobHash,
    size_bytes: u64,
    status: PutStatus,
}

impl PutResult {
    pub(crate) const fn new(hash: BlobHash, size_bytes: u64, status: PutStatus) -> Self {
        Self {
            hash,
            size_bytes,
            status,
        }
    }
    #[must_use]
    pub const fn hash(&self) -> BlobHash {
        self.hash
    }
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    #[must_use]
    pub const fn status(&self) -> PutStatus {
        self.status
    }
}

/// Metadata for one published blob.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BlobMetadata {
    hash: BlobHash,
    size_bytes: u64,
}

impl BlobMetadata {
    pub(crate) const fn new(hash: BlobHash, size_bytes: u64) -> Self {
        Self { hash, size_bytes }
    }
    #[must_use]
    pub const fn hash(&self) -> BlobHash {
        self.hash
    }
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

/// Outcome of a maintenance deletion.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DeleteOutcome {
    Deleted,
    NotFound,
}

/// Controls the read-time integrity/performance tradeoff.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum IntegrityMode {
    /// Recompute SHA-256 on every read and reject corruption.
    VerifyOnRead,
    /// Skip read-time hashing. Existing objects are still verified during puts.
    TrustStorage,
}

/// Configuration for a filesystem blob store.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct FsBlobStoreConfig {
    integrity_mode: IntegrityMode,
    max_concurrent_io: usize,
}

impl FsBlobStoreConfig {
    /// Builds validated configuration.
    ///
    /// # Errors
    /// Returns [`BlobStoreError::InvalidConfiguration`] when the I/O limit is zero.
    pub fn new(
        integrity_mode: IntegrityMode,
        max_concurrent_io: usize,
    ) -> Result<Self, BlobStoreError> {
        if max_concurrent_io == 0 {
            return Err(BlobStoreError::InvalidConfiguration {
                message: "max_concurrent_io must be greater than zero".to_owned(),
            });
        }
        Ok(Self {
            integrity_mode,
            max_concurrent_io,
        })
    }
    #[must_use]
    pub const fn development_default() -> Self {
        Self {
            integrity_mode: IntegrityMode::VerifyOnRead,
            max_concurrent_io: 32,
        }
    }
    #[must_use]
    pub const fn integrity_mode(&self) -> IntegrityMode {
        self.integrity_mode
    }
    #[must_use]
    pub const fn max_concurrent_io(&self) -> usize {
        self.max_concurrent_io
    }
}
