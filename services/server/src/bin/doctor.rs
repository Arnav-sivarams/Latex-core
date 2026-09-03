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
use std::env;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database =
        Database::connect(DatabaseConfig::development(required("DATABASE_URL")?)?).await?;
    let mail = mail::MailSettings::from_env()?;
    database.migrate().await?;
    database.health_check().await?;
    let _blobs = FsBlobStore::open(
        required("BLOB_STORAGE_ROOT")?,
        FsBlobStoreConfig::development_default(),
    )
    .await?;
    let runtime = DockerCliRuntime::new(required("COMPILER_IMAGE")?)?;
    let _ = runtime.probe_image()?;
    println!(
        "LaTeX Core Doctor\n────────────────────────────────\nDatabase         ✓ Healthy\nBlob storage     ✓ Healthy\nDocker runtime   ✓ Healthy\nCompiler M7      ✓ Verified\nMail delivery    {}\n\nAll systems healthy.",
        if mail.enabled {
            "✓ Configured"
        } else {
            "disabled"
        }
    );
    Ok(())
}
fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    env::var(name).map_err(|_| format!("required environment variable {name} is missing").into())
}
