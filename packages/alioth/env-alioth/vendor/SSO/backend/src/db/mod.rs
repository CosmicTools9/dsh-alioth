use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn new(config: &crate::Config) -> anyhow::Result<Self> {
        log::info!(
            "Creating database pool: max_connections=30, database_url={}",
            mask_database_url(&config.database_url)
        );
        let pool = PgPoolOptions::new()
            .max_connections(30)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect(&config.database_url)
            .await?;

        log::info!("Database pool created successfully");
        Ok(Self { pool })
    }

    /// 返回数据库连接池大小与空闲连接数，用于诊断连接池是否耗尽。
    pub fn pool_stats(&self) -> (u32, usize) {
        (self.pool.size(), self.pool.num_idle())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// 隐藏数据库 URL 中的密码，避免日志泄露敏感凭证。
fn mask_database_url(url: &str) -> String {
    if let Some(at) = url.find('@') {
        let prefix = &url[..at];
        if let Some(colon) = prefix.rfind(':') {
            // 确认前面有 `://`，避免把端口或 IPv6 误判为密码
            if prefix[..colon].contains("://") {
                return format!("{}:***@{}", &url[..colon], &url[at + 1..]);
            }
        }
    }
    url.to_string()
}
