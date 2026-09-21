use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
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

/// 连接级会话参数（经 libpq `options=-c k=v` 在**连接建立时**下发，池内每个连接都生效）。
///
/// `jit=off`：本平台查询是小数据量 OLTP（线上实测 consignments=18 行 / subjects=338 行），
/// 而 JIT 是**按次执行**付的编译开销——委托列表查询因估算爆炸
/// （`Sort (cost=36918260015.46..) rows=685107693`，实际 3 行）越过 `jit_above_cost=100000`，
/// 触发 1635 个函数的 LLVM 编译（含 inlining/optimization），单次执行从 49.9 ms 劣化到 17.8 s。
/// 库级 `ALTER DATABASE … SET jit = off` 会波及该库全部角色与工具（psql / pg_dump / reset 链），
/// 进程级（本池）更精准（openspec `gateway-infra-reliability::gateway-pool-jit-disabled`）。
pub const SESSION_GUCS: [(&str, &str); 1] = [("jit", "off")];

/// 由 `DATABASE_URL` 构造连接选项（含 [`SESSION_GUCS`]）；池创建唯一入口。
fn connect_options(database_url: &str) -> Result<PgConnectOptions, sqlx::Error> {
    Ok(database_url
        .parse::<PgConnectOptions>()?
        .options(SESSION_GUCS))
}

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn new(config: &crate::Config) -> anyhow::Result<Self> {
        let cfg = pool_config_from_env();
        // 连接选项由 connect_options 统一构造（含会话级 GUC，见 SESSION_GUCS）
        let opts = connect_options(&config.database_url)?;
        let pool = PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .acquire_timeout(Duration::from_secs(cfg.acquire_timeout_secs))
            // 与 Meta 侧同口径：空闲 10 分钟回收、最长存活 30 分钟，避免长连接陈旧
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(1800))
            .connect_with(opts)
            .await?;

        log::info!(
            "Database pool ready: max_connections={}, acquire_timeout={}s, session_gucs={:?}",
            cfg.max_connections,
            cfg.acquire_timeout_secs,
            SESSION_GUCS
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

    /// 行为契约：池连接**建立时**即关闭 JIT——不依赖库级/实例级参数。
    /// 理由见 [`SESSION_GUCS`]：估算爆炸查询每次都付 LLVM 编译费（实测 17.8 s vs 49.9 ms）。
    #[tokio::test]
    async fn pool_connections_start_with_jit_disabled() {
        let url = common::testing::test_database_url();
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(connect_options(&url).expect("parse test database url"))
            .await
            .expect("connect test database");
        let jit: String = sqlx::query_scalar("SHOW jit")
            .fetch_one(&pool)
            .await
            .expect("SHOW jit");
        assert_eq!(jit, "off", "池连接 MUST 以 jit=off 建立（会话级 GUC）");
        pool.close().await;
    }

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
