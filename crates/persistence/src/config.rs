//! `PostgreSQL` pool configuration.

use crate::PersistenceError;
use std::{fmt, time::Duration};

/// Validated database connection-pool configuration.
#[derive(Clone)]
pub struct DatabaseConfig {
    url: String,
    max_connections: u32,
    min_connections: u32,
    acquire_timeout: Duration,
}

impl DatabaseConfig {
    /// Creates validated configuration.
    ///
    /// # Errors
    /// Returns [`PersistenceError::InvalidConfiguration`] when a pool invariant is violated.
    pub fn new(
        url: impl Into<String>,
        min_connections: u32,
        max_connections: u32,
        acquire_timeout: Duration,
    ) -> Result<Self, PersistenceError> {
        let url = url.into();
        if url.is_empty() {
            return Err(invalid("database URL must not be empty"));
        }
        if max_connections == 0 {
            return Err(invalid("maximum connections must be greater than zero"));
        }
        if min_connections > max_connections {
            return Err(invalid(
                "minimum connections must not exceed maximum connections",
            ));
        }
        if acquire_timeout.is_zero() {
            return Err(invalid("acquire timeout must be greater than zero"));
        }
        Ok(Self {
            url,
            max_connections,
            min_connections,
            acquire_timeout,
        })
    }

    /// Creates the development-sized pool defaults for a caller-supplied URL.
    ///
    /// # Errors
    /// Returns [`PersistenceError::InvalidConfiguration`] if the URL is empty.
    pub fn development(url: impl Into<String>) -> Result<Self, PersistenceError> {
        Self::new(url, 1, 16, Duration::from_secs(5))
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
    #[must_use]
    pub const fn max_connections(&self) -> u32 {
        self.max_connections
    }
    #[must_use]
    pub const fn min_connections(&self) -> u32 {
        self.min_connections
    }
    #[must_use]
    pub const fn acquire_timeout(&self) -> Duration {
        self.acquire_timeout
    }
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatabaseConfig")
            .field("url", &"[REDACTED]")
            .field("max_connections", &self.max_connections)
            .field("min_connections", &self.min_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .finish()
    }
}

fn invalid(message: &str) -> PersistenceError {
    PersistenceError::InvalidConfiguration {
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_configuration() {
        assert!(DatabaseConfig::new("postgresql://example", 1, 4, Duration::from_secs(1)).is_ok());
        assert!(DatabaseConfig::new("", 1, 4, Duration::from_secs(1)).is_err());
        assert!(DatabaseConfig::new("x", 0, 0, Duration::from_secs(1)).is_err());
        assert!(DatabaseConfig::new("x", 5, 4, Duration::from_secs(1)).is_err());
        assert!(DatabaseConfig::new("x", 1, 4, Duration::ZERO).is_err());
    }

    #[test]
    fn development_defaults_are_bounded() {
        let config = DatabaseConfig::development("postgresql://example")
            .expect("known-valid development configuration");
        assert_eq!(config.min_connections(), 1);
        assert_eq!(config.max_connections(), 16);
        assert_eq!(config.acquire_timeout(), Duration::from_secs(5));
        assert!(!format!("{config:?}").contains("postgresql://example"));
    }
}
