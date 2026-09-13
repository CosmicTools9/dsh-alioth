use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// 连接池参数（ENVIRONMENT_SPEC §7.3 声明的 `DATABASE_POOL_SIZE` / `DATABASE_TIMEOUT`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolConfig {
    /// `max_connections`——连接池上限。
    pub max_connections: u32,
    /// `acquire_timeout`——单次取连接的最长等待（秒）。
    pub acquire_timeout_secs: u64,
}

impl PoolConfig {
    /// 声明缺省值（ENVIRONMENT_SPEC §7.3：10 / 30）——环境变量缺失或非法时回落。
    pub const DEFAULT: Self = Self {
        max_connections: 10,
        acquire_timeout_secs: 30,
    };
}

/// 解析池参数原始值（env 为 `None` 或非正整数 → 回落声明缺省值）。
///
/// 抽为纯函数以便单测（env 是进程级全局态，测试直接注入原始值）。
pub fn resolve_pool_config(pool_size_raw: Option<&str>, timeout_raw: Option<&str>) -> PoolConfig {
    fn parse_positive<T: std::str::FromStr + PartialOrd + From<u8>>(
        raw: Option<&str>,
    ) -> Option<T> {
        raw.map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<T>().ok())
            .filter(|v| *v > T::from(0u8))
    }

    PoolConfig {
        max_connections: parse_positive::<u32>(pool_size_raw)
            .unwrap_or(PoolConfig::DEFAULT.max_connections),
        acquire_timeout_secs: parse_positive::<u64>(timeout_raw)
            .unwrap_or(PoolConfig::DEFAULT.acquire_timeout_secs),
    }
}

fn pool_config_from_env() -> PoolConfig {
    let pool_size = std::env::var("DATABASE_POOL_SIZE").ok();
    let timeout = std::env::var("DATABASE_TIMEOUT").ok();
    resolve_pool_config(pool_size.as_deref(), timeout.as_deref())
}

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn new(config: &crate::Config) -> anyhow::Result<Self> {
        let cfg = pool_config_from_env();
        let pool = PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .acquire_timeout(Duration::from_secs(cfg.acquire_timeout_secs))
            // 与 Meta 侧同口径：空闲 10 分钟回收、最长存活 30 分钟，避免长连接陈旧
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(1800))
            .connect(&config.database_url)
            .await?;

        log::info!(
            "Database pool ready: max_connections={}, acquire_timeout={}s",
            cfg.max_connections,
            cfg.acquire_timeout_secs
        );

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_pool_config_env_values_honored() {
        let cfg = resolve_pool_config(Some("20"), Some("3"));
        assert_eq!(cfg.max_connections, 20);
        assert_eq!(cfg.acquire_timeout_secs, 3);
    }

    #[test]
    fn resolve_pool_config_missing_falls_back_to_declared_defaults() {
        let cfg = resolve_pool_config(None, None);
        assert_eq!(cfg, PoolConfig::DEFAULT);
        assert_eq!(cfg.max_connections, 10);
        assert_eq!(cfg.acquire_timeout_secs, 30);
    }

    #[test]
    fn resolve_pool_config_invalid_or_non_positive_falls_back() {
        assert_eq!(
            resolve_pool_config(Some("abc"), Some("")),
            PoolConfig::DEFAULT
        );
        assert_eq!(
            resolve_pool_config(Some("0"), Some("0")),
            PoolConfig::DEFAULT
        );
        assert_eq!(
            resolve_pool_config(Some("  "), Some("-5")),
            PoolConfig::DEFAULT
        );
        // 空白填充的合法值仍生效（env 常见写法）
        assert_eq!(
            resolve_pool_config(Some(" 16 "), Some(" 5 ")),
            PoolConfig {
                max_connections: 16,
                acquire_timeout_secs: 5,
            }
        );
    }
}
