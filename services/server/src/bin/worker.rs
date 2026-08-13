//! Worker-only compiler entry point. Deploy this binary with Docker socket access, never the API.
#![forbid(unsafe_code)]

use blob_store::{FsBlobStore, FsBlobStoreConfig};
use compiler::{CompileLimits, CompilerConfig, CompilerService, DockerCliRuntime};
use core_types::TexEnvironmentId;
use persistence::{Database, DatabaseConfig, PostgresCompileQueue, QueueLimits};
use queue::{CompilationWorker, WorkerConfig, WorkerShutdown};
use std::{env, path::PathBuf, sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let database =
        Database::connect(DatabaseConfig::development(required("DATABASE_URL")?)?).await?;
    database.migrate().await?;
    let blobs = Arc::new(
        FsBlobStore::open(
            required("BLOB_STORAGE_ROOT")?,
            FsBlobStoreConfig::development_default(),
        )
        .await?,
    );
    let runtime = DockerCliRuntime::new(required("COMPILER_IMAGE")?)?;
    let reaped = runtime.reap_orphans()?;
    tracing::info!(reaped, "reaped stale labelled compiler containers");
    let staging = PathBuf::from(required("WORKER_STAGING_ROOT")?);
    let compiler = CompilerService::new(
        blobs.clone(),
        runtime,
        CompilerConfig::new(CompileLimits::development_default()).with_staging_root(staging),
    )?;
    let configured_environment = TexEnvironmentId::parse(&required("TEX_ENVIRONMENT_ID")?)?;
    if compiler.environment_id() != &configured_environment {
        return Err(format!(
            "configured TEX_ENVIRONMENT_ID {} does not match compiler image {}",
            configured_environment,
            compiler.environment_id()
        )
        .into());
    }
    let queue = PostgresCompileQueue::new(database, queue_limits()?);
    let recovered = queue.recover_expired_leases().await?;
    tracing::info!(recovered, "recovered expired compile leases");
    let worker = CompilationWorker::new(
        queue,
        blobs,
        Arc::new(compiler),
        core_types::WorkerId::new(),
        WorkerConfig::new(
            usize::try_from(int_env("WORKER_CONCURRENCY", 1)?)?,
            Duration::from_millis(250),
        )?,
    );
    let shutdown = WorkerShutdown::new();
    let signal = shutdown.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        signal.request();
    });
    worker.run_until_shutdown(shutdown).await?;
    Ok(())
}
fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    env::var(name).map_err(|_| format!("required environment variable {name} is missing").into())
}
fn int_env(name: &str, default: i64) -> Result<i64, Box<dyn std::error::Error>> {
    match env::var(name) {
        Ok(v) => {
            let n = v.parse()?;
            if n <= 0 {
                return Err(format!("{name} must be positive").into());
            }
            Ok(n)
        }
        Err(_) => Ok(default),
    }
}
fn queue_limits() -> Result<QueueLimits, Box<dyn std::error::Error>> {
    QueueLimits::new(
        u32::try_from(int_env("QUEUE_GLOBAL_RUNNING", 2)?)?,
        u32::try_from(int_env("QUEUE_PER_USER_RUNNING", 1)?)?,
        u32::try_from(int_env("QUEUE_PER_USER_OUTSTANDING", 8)?)?,
        Duration::from_secs(u64::try_from(int_env("QUEUE_LEASE_SECONDS", 120)?)?),
        u32::try_from(int_env("QUEUE_MAX_ATTEMPTS", 3)?)?,
    )
    .map_err(Into::into)
}
