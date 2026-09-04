//! Query logger for database performance monitoring
//! 
//! Logs slow queries that exceed the configured threshold.
//! Phase 18: Performance Optimization (PERF-01)

use std::time::Duration;
use common::telemetry::{debug, warn};

/// Configuration for query logging
#[derive(Debug, Clone)]
pub struct QueryLoggerConfig {
    /// Threshold in milliseconds for slow query logging
    pub threshold_ms: u64,
    /// Enable debug logging for all queries
    pub log_all: bool,
}

impl Default for QueryLoggerConfig {
    fn default() -> Self {
        Self {
            threshold_ms: 100, // 100ms default threshold
            log_all: false,
        }
    }
}

/// Query logger for tracking database performance
#[derive(Debug, Clone)]
pub struct QueryLogger {
    config: QueryLoggerConfig,
}

impl QueryLogger {
    /// Create a new query logger with default config
    pub fn new() -> Self {
        Self::with_config(QueryLoggerConfig::default())
    }

    /// Create a new query logger with custom config
    pub fn with_config(config: QueryLoggerConfig) -> Self {
        Self { config }
    }

    /// Log a query with its execution duration
    /// 
    /// - Queries exceeding threshold are logged as warnings
    /// - Other queries are logged as debug (if log_all is true)
    pub fn log(&self, query: &str, duration: Duration) {
        let duration_ms = duration.as_millis() as u64;

        if duration_ms >= self.config.threshold_ms {
            warn!(
                target: "db.slow_query",
                duration_ms = duration_ms,
                threshold_ms = self.config.threshold_ms,
                query = truncate_query(query, 500),
                "Slow query detected"
            );
        } else if self.config.log_all {
            debug!(
                target: "db.query",
                duration_ms = duration_ms,
                query = truncate_query(query, 200),
            );
        }
    }

    /// Get the slow query threshold in milliseconds
    pub fn threshold_ms(&self) -> u64 {
        self.config.threshold_ms
    }
}

impl Default for QueryLogger {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate long queries for logging
fn truncate_query(query: &str, max_len: usize) -> &str {
    if query.len() <= max_len {
        query
    } else {
        &query[..max_len.saturating_sub(3)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short_query() {
        let query = "SELECT * FROM users";
        assert_eq!(truncate_query(query, 50), query);
    }

    #[test]
    fn test_truncate_long_query() {
        let query = "SELECT ".repeat(100);
        let truncated = truncate_query(&query, 50);
        assert!(truncated.len() <= 50);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn test_default_threshold() {
        let logger = QueryLogger::new();
        assert_eq!(logger.threshold_ms(), 100);
    }
}
