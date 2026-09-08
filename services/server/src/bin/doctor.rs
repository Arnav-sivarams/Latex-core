//! Small release readiness check; run where the worker's Docker socket is available.
#![forbid(unsafe_code)]
#[path = "../mail.rs"]
#[allow(
    dead_code,
    reason = "doctor shares only strict mail configuration validation"
)]
mod mail;
use blob_store::{FsBlobStore, FsBlobStoreConfig};
use compiler::{ContainerRuntime, DockerCliRuntime};
use persistence::{Database, DatabaseConfig};
use std::{env, fs, path::Path};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database =
        Database::connect(DatabaseConfig::development(required("DATABASE_URL")?)?).await?;
    let mail = mail::MailSettings::from_env()?;
    database.health_check().await?;
    let _blobs = FsBlobStore::open(
        required("BLOB_STORAGE_ROOT")?,
        FsBlobStoreConfig::development_default(),
    )
    .await?;
    let runtime = DockerCliRuntime::new(required("COMPILER_IMAGE")?)?;
    let _ = runtime.probe_image()?;
    verify_staging(Path::new(&required("WORKER_STAGING_ROOT")?))?;
    println!(
        "LaTeX Core Doctor\n────────────────────────────────\nDatabase         ✓ Healthy\nBlob storage     ✓ Healthy\nWorker staging   ✓ Writable\nDocker runtime   ✓ Healthy\nCompiler M7      ✓ Verified\nMail delivery    {}\n\nAll systems healthy.",
        if mail.enabled {
            "✓ Configured"
        } else {
            "disabled"
        }
    );
    Ok(())
}
fn verify_staging(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(path)?;
    let probe = path.join(format!(
        ".latex-core-doctor-write-check-{}",
        std::process::id()
    ));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    fs::remove_file(probe)?;
    Ok(())
}
fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    env::var(name).map_err(|_| format!("required environment variable {name} is missing").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_probe_accepts_directory_and_rejects_regular_file() {
        let directory = tempfile::tempdir().unwrap();
        verify_staging(directory.path()).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(verify_staging(file.path()).is_err());
    }
}
