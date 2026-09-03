//! 测试基础设施：共享的测试数据库连接与清理函数。
//!
//! WZ、Alioth 等 namespace 的集成测试通过 `helpers::setup()` 模式使用此模块，
//! 而非在每个 service 的 `tests/` 下复制一份连接逻辑。
//! 这符合项目「一次定义，到处使用」的框架规约。
//!
//! # 测试库强制隔离（harden-test-db-isolation）
//!
//! 所有连接入口强制目标库为 `*_test`：`test_database_url()` 在 URL 层校验
//! （库名非 `*_test` 结尾即 panic），`connect_test_db()` 在连接层再校验
//! （`SELECT current_database()`）。运行环境 `DATABASE_URL` 指向 dev/生产库
//! 时测试立即失败——杜绝测试 DDL/DML 污染非测试库（实测事故：
//! app-agent 集成测试曾把 `zc_id_alioth-e2e` 建进 dev 库）。
#![doc(hidden)]

use sqlx::PgPool;
use url::Url;

/// 校验测试库 URL：库名必须以 `_test` 结尾，否则 panic。
///
/// URL 解析用 `url` crate（NO_REGEX 规约：结构化格式用解析器）。
fn assert_test_db_url(url: &str) {
    let parsed = Url::parse(url).expect("test database URL 解析失败（应为 postgres://... 格式）");
    let db = parsed
        .path_segments()
        .and_then(|mut segs| segs.next_back())
        .unwrap_or("");
    if !db.ends_with("_test") {
        panic!(
            "测试禁止连接非测试库：目标库 '{db}'（URL: {url}）。\
             测试数据库连接 MUST 指向 *_test 库（四层隔离）。\
             修正：DATABASE_URL=postgres://<user>@localhost:5432/aliothstudio_test \
             或取消 DATABASE_URL 使用默认测试库。"
        );
    }
}

/// 测试数据库 URL：优先 `DATABASE_URL`（必须指向 `*_test` 库，否则 panic），
/// fallback 到规范本地测试库 `postgres://<OS 用户>@localhost:5432/aliothstudio_test`
/// （与 scripts/test-all.sh ③ 一致）。
///
/// 注：无用户的 `postgres://localhost/...` 会被 sqlx 解析为 `anonymous` 角色导致连接失败，
/// 因此必须显式携带当前 OS 用户名（`whoami`）。
pub fn test_database_url() -> String {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        if !url.is_empty() {
            // sqlx 对无用户名的 URL（postgres://host/db）解析为 `anonymous` 角色，
            // 且**不读取 PGUSER**（libpq/psql 才回退 PGUSER）——此处显式注入 OS 用户。
            // 无 user 的 URL 不含 '@'；含 '@' 说明已有 user[:pass]，不触碰。
            let url = if !url.contains('@') {
                let user = std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "postgres".to_string());
                if let Some(stripped) = url.strip_prefix("postgres://") {
                    format!("postgres://{}@{}", user, stripped)
                } else {
                    url
                }
            } else {
                url
            };
            assert_test_db_url(&url);
            return url;
        }
    }
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "postgres".to_string());
    format!("postgres://{}@localhost:5432/aliothstudio_test", user)
}

/// 连接测试数据库。
///
/// 优先读取 `DATABASE_URL` 环境变量（必须指向 `*_test` 库），fallback 到规范本地
/// 测试库（见 [`test_database_url`]）。连接后校验 `current_database()` 为 `*_test`，
/// 防 URL 混淆/服务端默认库绕过。
pub async fn connect_test_db() -> PgPool {
    let database_url = test_database_url();
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("Failed to read current_database()");
    if !db_name.ends_with("_test") {
        panic!(
            "connect_test_db 连接后校验失败：实际落在非测试库 '{db_name}'（URL: {database_url}）。\
             测试数据库连接 MUST 指向 *_test 库。"
        );
    }
    pool
}

/// 尝试连接测试数据库（跳过模式）。
///
/// 供"DB 不可用则跳过"测试使用：URL 校验与 [`connect_test_db`] 一致（非 `*_test`
/// 库同样 panic，不豁免），仅连接失败返回 `None`（测试跳过）。
pub async fn try_connect_test_db() -> Option<PgPool> {
    let database_url = test_database_url();
    let pool = PgPool::connect(&database_url).await.ok()?;
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .ok()?;
    if !db_name.ends_with("_test") {
        panic!(
            "try_connect_test_db 连接后校验失败：实际落在非测试库 '{db_name}'（URL: {database_url}）。\
             测试数据库连接 MUST 指向 *_test 库。"
        );
    }
    Some(pool)
}

/// 清理测试数据。
///
/// 当前实现为空操作。测试应通过独立的种子/回滚策略管理各自的数据生命周期。
/// 此函数作为扩展点预留，后续可接入 schema-level 清理逻辑。
pub async fn cleanup_test_db(_pool: &PgPool) {
    // no-op: 测试自己管理数据的创建与清理
}

/// 开始测试事务。事务结束时自动回滚，不影响其他测试。
pub async fn begin_test_tx(pool: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    pool.begin()
        .await
        .expect("Failed to begin test transaction")
}

/// 测试 schema 守门：断言当前连接是测试库。
///
/// 仅检查库名含 `_test`，不执行 TRUNCATE。
/// Alioth namespace 可通过自己的测试 helper 实现 TRUNCATE。
pub async fn setup_test_schema_light(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await?;
    if !db_name.contains("_test") {
        return Err(format!(
            "test db required (got '{}'); set DATABASE_URL={}",
            db_name,
            test_database_url()
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_rejects_non_test_db() {
        // harden-test-db-isolation：非 *_test 库必须 panic
        let r = std::panic::catch_unwind(|| {
            assert_test_db_url("postgres://isahl@localhost:5432/aliothstudio_dev")
        });
        assert!(r.is_err(), "非 *_test 库 URL 必须 panic");

        let r = std::panic::catch_unwind(|| {
            assert_test_db_url("postgres://isahl@localhost:5432/aliothstudio")
        });
        assert!(r.is_err(), "生产库名 URL 必须 panic");
    }

    #[test]
    fn test_url_accepts_test_db() {
        assert_test_db_url("postgres://isahl@localhost:5432/aliothstudio_test");
        assert_test_db_url("postgres://isahl@localhost:5432/wz_test?sslmode=disable");
        assert_test_db_url("postgres://isahl@localhost:5432/ns_test");
    }
}
