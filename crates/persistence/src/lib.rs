//! Structured persistence boundaries.
#![forbid(unsafe_code)]

mod config;
mod database;
mod error;

pub use config::DatabaseConfig;
pub use database::Database;
pub use error::PersistenceError;
