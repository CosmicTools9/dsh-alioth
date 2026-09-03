//! ZUID (Unique ID) Database Initialization Module
//!
//! At startup, reads IDC/CLUSTER/NODE from environment and rewrites the
//! `isahl.gen_next_zuid()` PostgreSQL function so its idc/cluster/node
//! parameters reflect the deployment topology.
//!
//! peer_type is ALWAYS 2 (Provider) — the only valid type for lifecycle
//! tables. It is NOT configurable from environment.
//!
//! Environment variables:
//! - ZUID_IDC:      IDC identifier (0-7),           default 0
//! - ZUID_CLUSTER:  Cluster identifier (0-7),       default 0
//! - ZUID_NODE:     Node identifier (0-31),          default 0

use sqlx::{AssertSqlSafe, PgPool};
use std::env;

#[derive(Debug, Clone)]
pub struct ZuidConfig {
    pub peer_type: i32,
    pub idc: i32,
    pub cluster: i32,
    pub node: i32,
}

impl Default for ZuidConfig {
    fn default() -> Self {
        Self {
            peer_type: 2,
            idc: 0,
            cluster: 0,
            node: 0,
        }
    }
}

impl ZuidConfig {
    pub fn from_env() -> Self {
        // peer_type is ALWAYS 2 (Provider) for lifecycle tables.
        // Only idc/cluster/node are configurable from environment.
        let idc = env::var("ZUID_IDC")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let cluster = env::var("ZUID_CLUSTER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let node = env::var("ZUID_NODE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        let config = Self {
            peer_type: 2,
            idc: idc.clamp(0, 7),
            cluster: cluster.clamp(0, 7),
            node: node.clamp(0, 31),
        };

        common::telemetry::info!(
            "ZUID config loaded: peer_type={}, idc={}, cluster={}, node={}",
            config.peer_type,
            config.idc,
            config.cluster,
            config.node
        );

        config
    }
}

pub fn validate_zuid_config(config: &ZuidConfig) -> Result<(), String> {
    if config.peer_type != 2 {
        return Err(format!(
            "ZUID peer_type must be 2 (Provider), got {}",
            config.peer_type
        ));
    }
    if config.idc < 0 || config.idc > 7 {
        return Err(format!(
            "ZUID_IDC must be between 0 and 7, got {}",
            config.idc
        ));
    }
    if config.cluster < 0 || config.cluster > 7 {
        return Err(format!(
            "ZUID_CLUSTER must be between 0 and 7, got {}",
            config.cluster
        ));
    }
    if config.node < 0 || config.node > 31 {
        return Err(format!(
            "ZUID_NODE must be between 0 and 31, got {}",
            config.node
        ));
    }
    Ok(())
}

pub async fn init_zuid_function(pool: &PgPool) -> Result<(), sqlx::Error> {
    let config = ZuidConfig::from_env();

    if let Err(e) = validate_zuid_config(&config) {
        common::telemetry::error!("ZUID configuration invalid: {}", e);
        return Err(sqlx::Error::Protocol(e));
    }

    // Build the expected function body for comparison
    let expected_body = format!(
        "BEGIN\n    RETURN isahl.gen_zuid(2, {}, {}, {});\nEND;",
        config.idc, config.cluster, config.node
    );

    // Check current function body — best-effort; on query failure fall through to DDL
    match check_current_zuid_body(pool).await {
        Ok(Some(ref body)) if equivalent_body(body, &expected_body) => {
            common::telemetry::info!("ZUID function already up-to-date: gen_next_zuid() → gen_zuid(2, {}, {}, {}) (skipped)",
            config.idc,
            config.cluster,
            config.node);
            return Ok(());
        }
        Ok(Some(_)) => {
            common::telemetry::info!(
                "ZUID function body changed, updating: gen_next_zuid() → gen_zuid(2, {}, {}, {})",
                config.idc,
                config.cluster,
                config.node
            );
        }
        Ok(None) => {
            common::telemetry::info!(
                "ZUID function does not exist, creating: gen_next_zuid() → gen_zuid(2, {}, {}, {})",
                config.idc,
                config.cluster,
                config.node
            );
        }
        Err(e) => {
            common::telemetry::warn!(
                "Failed to check current ZUID function body (will recreate anyway): {}",
                e
            );
        }
    }

    // DDL statement with retry for concurrent catalog updates
    let sql = format!(
        r#"
        CREATE OR REPLACE FUNCTION isahl.gen_next_zuid()
        RETURNS BIGINT
        LANGUAGE plpgsql
        AS $$
        BEGIN
            RETURN isahl.gen_zuid(2, {}, {}, {});
        END;
        $$;
        "#,
        config.idc, config.cluster, config.node
    );

    let mut last_err = None;
    for attempt in 0..3 {
        match sqlx::query(AssertSqlSafe(sql.as_str())).execute(pool).await {
            Ok(_) => {
                common::telemetry::info!(
                    "ZUID function updated: gen_next_zuid() → gen_zuid(2, {}, {}, {})",
                    config.idc,
                    config.cluster,
                    config.node
                );
                return Ok(());
            }
            Err(e) if is_concurrent_update_error(&e) && attempt < 2 => {
                last_err = Some(e);
                let delay_ms = 50u64 * (attempt as u64 + 1); // 50ms, 100ms
                common::telemetry::warn!(
                    "ZUID DDL concurrent update (attempt {}), retrying in {}ms...",
                    attempt + 1,
                    delay_ms
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(e) => return Err(e),
        }
    }

    // Unreachable — the loop always returns or panics. Appease the compiler.
    Err(last_err.unwrap())
}

/// Query the current body of `isahl.gen_next_zuid()` from the system catalog.
async fn check_current_zuid_body(pool: &PgPool) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT prosrc::text FROM pg_proc
        WHERE proname = 'gen_next_zuid'
          AND pronamespace = (SELECT oid FROM pg_namespace WHERE nspname = 'isahl')
        "#,
    )
    .fetch_optional(pool)
    .await
}

/// Are two function bodies semantically equivalent (normalized whitespace)?
fn equivalent_body(a: &str, b: &str) -> bool {
    // Normalize: collapse runs of whitespace, strip leading/trailing, ignore semicolons-only diffs
    fn normalize(s: &str) -> String {
        s.split_whitespace()
            .filter(|w| *w != ";")
            .collect::<Vec<_>>()
            .join(" ")
    }
    normalize(a) == normalize(b)
}

/// Does the error indicate a concurrent catalog tuple update?
fn is_concurrent_update_error(e: &sqlx::Error) -> bool {
    matches!(
        e,
        sqlx::Error::Database(db_err)
            if db_err.code().as_deref() == Some("XX000")
                && db_err.message().contains("tuple concurrently updated")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zuid_config_default() {
        let config = ZuidConfig::default();
        assert_eq!(config.peer_type, 2);
        assert_eq!(config.idc, 0);
        assert_eq!(config.cluster, 0);
        assert_eq!(config.node, 0);
    }

    #[test]
    fn test_zuid_config_validation() {
        let valid = ZuidConfig {
            peer_type: 2,
            idc: 5,
            cluster: 3,
            node: 15,
        };
        assert!(validate_zuid_config(&valid).is_ok());

        let invalid_peer_type = ZuidConfig {
            peer_type: 5,
            idc: 0,
            cluster: 0,
            node: 0,
        };
        assert!(validate_zuid_config(&invalid_peer_type).is_err());

        let invalid_node = ZuidConfig {
            peer_type: 2,
            idc: 0,
            cluster: 0,
            node: 50,
        };
        assert!(validate_zuid_config(&invalid_node).is_err());
    }
}
