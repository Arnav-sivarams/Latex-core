use crate::CompilerError;
use blob_store::{BlobStore, BlobStoreError};
use core_types::{BlobHash, SnapshotId, WorkspaceManifestV1};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{fs, io::Write, path::Path, sync::Arc};

pub(crate) async fn materialize(
    root: &Path,
    requested: SnapshotId,
    manifest: &WorkspaceManifestV1,
    blobs: &Arc<dyn BlobStore>,
) -> Result<(), CompilerError> {
    let actual_snapshot =
        manifest
            .snapshot_id()
            .map_err(|error| CompilerError::InternalInvariant {
                message: error.to_string(),
            })?;
    if actual_snapshot != requested {
        return Err(CompilerError::SnapshotMismatch {
            requested,
            actual: actual_snapshot,
        });
    }
    for (logical, entry) in manifest.files() {
        let bytes = match blobs.get(entry.blob_hash).await {
            Ok(value) => value,
            Err(BlobStoreError::NotFound { .. }) => {
                return Err(CompilerError::MissingBlob {
                    hash: entry.blob_hash,
                });
            }
            Err(error) => {
                return Err(CompilerError::ContainerInfrastructure {
                    message: format!("blob read failed: {error}"),
                });
            }
        };
        let actual = BlobHash::digest(&bytes);
        if actual != entry.blob_hash {
            return Err(CompilerError::BlobIntegrityMismatch {
                expected: entry.blob_hash,
                actual,
            });
        }
        let size = u64::try_from(bytes.len()).map_err(|_| CompilerError::InternalInvariant {
            message: "blob length does not fit u64".into(),
        })?;
        if size != entry.size_bytes {
            return Err(CompilerError::Materialization {
                expected: entry.size_bytes,
                actual: size,
            });
        }
        let destination = root.join(logical.as_str());
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| CompilerError::Io {
                operation: "create materialization directory",
                source,
            })?;
            set_mode(parent, 0o755)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|source| CompilerError::Io {
                operation: "create materialized regular file",
                source,
            })?;
        file.write_all(&bytes).map_err(|source| CompilerError::Io {
            operation: "write materialized file",
            source,
        })?;
        set_mode(&destination, 0o444)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), CompilerError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
        CompilerError::Io {
            operation: "set materialized permissions",
            source,
        }
    })
}
