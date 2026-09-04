//! customer service 集成测试共享辅助（与 demand tests/common 同约定，per-crate 复制）。
use sqlx::PgPool;

/// 生成唯一测试标识（避免测试间数据冲突）
pub fn test_code(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{}",
        prefix,
        std::process::id(),
        nanos % 1_000_000_000
    )
}

/// 测试 schema 守门：断言当前连接是测试库且 isahl schema 已初始化
pub async fn setup_test_schema(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await?;
    if !db_name.contains("_test") {
        return Err(format!(
            "REFUSED: running integration test on non-test database '{}'. \
             Set DATABASE_URL to aliothstudio_test before running tests.",
            db_name,
        )
        .into());
    }
    sqlx::query("CREATE SCHEMA IF NOT EXISTS isahl")
        .execute(pool)
        .await?;
    Ok(())
}
