//! `PostgreSQL` connection and embedded migration lifecycle.

use crate::{DatabaseConfig, PersistenceError};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::fmt;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Cloneable `PostgreSQL` database handle.
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl fmt::Debug for Database {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Database")
            .field("closed", &self.pool.is_closed())
            .finish_non_exhaustive()
    }
}

impl Database {
    /// Connects a bounded pool and initializes every connection's session state.
    ///
    /// # Errors
    /// Returns [`PersistenceError::Database`] when the pool cannot connect or initialize.
    pub async fn connect(config: DatabaseConfig) -> Result<Self, PersistenceError> {
        let pool = PgPoolOptions::new()
            .min_connections(config.min_connections())
            .max_connections(config.max_connections())
            .acquire_timeout(config.acquire_timeout())
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    sqlx::query("SET search_path TO latex_core, pg_catalog")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SET TIME ZONE 'UTC'")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(config.url())
            .await
            .map_err(PersistenceError::Database)?;
        Ok(Self { pool })
    }

    /// Runs migrations embedded in the production binary.
    ///
    /// # Errors
    /// Returns [`PersistenceError::Migration`] when an embedded migration fails.
    pub async fn migrate(&self) -> Result<(), PersistenceError> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(PersistenceError::Database)?;
        // SQLx creates its history table before the first migration can create latex_core.
        sqlx::query("SET search_path TO public, pg_catalog")
            .execute(&mut *connection)
            .await
            .map_err(PersistenceError::Database)?;
        let migration_result = MIGRATOR.run(&mut *connection).await;
        let restore_result = sqlx::query("SET search_path TO latex_core, pg_catalog")
            .execute(&mut *connection)
            .await;
        migration_result.map_err(PersistenceError::Migration)?;
        restore_result.map_err(PersistenceError::Database)?;
        Ok(())
    }

    /// Executes a minimal round trip through the pool.
    ///
    /// # Errors
    /// Returns [`PersistenceError::Database`] when the health query fails.
    pub async fn health_check(&self) -> Result<(), PersistenceError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(PersistenceError::Database)
    }

    /// Gracefully closes the pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    #[allow(dead_code, reason = "reserved for persistence repositories")]
    pub(crate) const fn pool(&self) -> &PgPool {
        &self.pool
    }
}
