#![forbid(unsafe_code)]
//! Immutable, content-addressed binary object storage.

mod error;
mod fs;
mod store;
mod types;

pub use error::BlobStoreError;
pub use fs::FsBlobStore;
pub use store::{BlobStore, BlobStoreMaintenance};
pub use types::{
    BlobMetadata, DeleteOutcome, FsBlobStoreConfig, IntegrityMode, PutResult, PutStatus,
};
