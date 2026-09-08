//! Worker-only compiler entry point. Deploy this binary with Docker socket access, never the API.
#![forbid(unsafe_code)]

#[path = "../mail.rs"]
#[allow(
    dead_code,
    reason = "worker does not construct API repository wrappers"
)]
mod mail;

use blob_store::{FsBlobStore, FsBlobStoreConfig};
use compiler::{CompileLimits, CompilerConfig, CompilerService, DockerCliRuntime};
use core_types::TexEnvironmentId;
use persistence::{Database, DatabaseConfig, PostgresCompileQueue, QueueLimits};
use queue::{CompilationWorker, WorkerConfig, WorkerShutdown};
use std::{env, fs, path::PathBuf, sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let database =
        Database::connect(DatabaseConfig::development(required("DATABASE_URL")?)?).await?;
    database.migrate().await?;
    let mail_settings = mail::MailSettings::from_env()?;
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
    verify_staging(&staging)?;
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
    let queue = PostgresCompileQueue::new(database.clone(), queue_limits()?);
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
    let mail_task = if mail_settings.enabled {
        let repository = mail_settings
            .repository(database.clone())
            .ok_or("mail outbox configuration is missing")?;
        let transport = Arc::new(mail::SmtpMailTransport::new(&mail_settings)?);
        let mail_shutdown = shutdown.clone();
        tokio::spawn(mail::run_mail_worker(
            repository,
            transport,
            mail_settings,
            mail_shutdown,
        ))
    } else {
        tracing::info!("mail delivery disabled");
        let mail_shutdown = shutdown.clone();
        tokio::spawn(mail::run_disabled_mail_maintenance(database, mail_shutdown))
    };
    let compile_result = worker.run_until_shutdown(shutdown.clone()).await;
    shutdown.request();
    mail_task.await??;
    compile_result?;
    Ok(())
}
fn verify_staging(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(path)?;
    let probe = path.join(format!(
        ".latex-core-worker-write-check-{}",
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
