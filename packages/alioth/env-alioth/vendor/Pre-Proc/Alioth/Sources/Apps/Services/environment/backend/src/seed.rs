//! 运行环境种子数据
//!
//! 为 system-settings 模块提供通用运行环境（dev / test / staging / prod / dr）。
//! 幂等执行：按 `code`（host）去重；依赖 `zc_id_status` 中的 healthy / warning / unknown 记录。

use common::error::AliothError;
use sqlx::PgPool;

const SEED_USER_ID: i64 = 1;

#[derive(Debug, Clone)]
struct SeedEnvironment {
    name: &'static str,
    host: &'static str,
    os: &'static str,
    runtime: &'static str,
    type_: &'static str,
    status: &'static str,
    services: i32,
    uptime: &'static str,
    comments: &'static str,
}

fn all_seed_environments() -> Vec<SeedEnvironment> {
    vec![
        SeedEnvironment {
            name: "本地开发",
            host: "localhost:3000",
            os: "macOS 15.4",
            runtime: "Node 22",
            type_: "dev",
            status: "healthy",
            services: 3,
            uptime: "72h",
            comments: "本地开发环境",
        },
        SeedEnvironment {
            name: "CI/CD 测试",
            host: "ci.alioth.dev",
            os: "Ubuntu 24.04",
            runtime: "Node 22",
            type_: "test",
            status: "healthy",
            services: 5,
            uptime: "12h",
            comments: "持续集成测试环境",
        },
        SeedEnvironment {
            name: "预发布",
            host: "staging.alioth.dev",
            os: "Debian 12",
            runtime: "Node 20",
            type_: "staging",
            status: "warning",
            services: 4,
            uptime: "168h",
            comments: "预发布验证环境",
        },
        SeedEnvironment {
            name: "生产环境",
            host: "prod.alioth.dev",
            os: "Debian 12",
            runtime: "Node 20",
            type_: "prod",
            status: "healthy",
            services: 8,
            uptime: "720h",
            comments: "生产运行环境",
        },
        SeedEnvironment {
            name: "灾备环境",
            host: "dr.alioth.dev",
            os: "Debian 12",
            runtime: "Node 18",
            type_: "dr",
            status: "unknown",
            services: 2,
            uptime: "N/A",
            comments: "灾难恢复环境",
        },
    ]
}

async fn ensure_status(
    pool: &PgPool,
    notice: &str,
    flag: &str,
    comments: &str,
) -> Result<i64, AliothError> {
    let id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl.zc_id_status
           WHERE notice = $1 AND deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(notice)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)?;

    if let Some(id) = id {
        return Ok(id);
    }

    sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_status (notice, flag, comments, created_by_id)
           VALUES ($1, $2::isahl.status_flag, $3, $4)
           RETURNING id"#,
    )
    .bind(notice)
    .bind(flag)
    .bind(comments)
    .bind(SEED_USER_ID)
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)
}

/// 向数据库预置通用运行环境记录。
///
/// 按 `code`（host）去重；已存在非删除记录则跳过，保证幂等。
/// 会自动创建缺少的 `zc_id_status` 记录（healthy / warning / unknown）。
pub async fn seed_environments(pool: &PgPool) -> Result<usize, AliothError> {
    let status_map = std::collections::HashMap::from([
        ("healthy", ("start", "正常运行状态")),
        ("warning", ("doing", "需要关注状态")),
        ("unknown", ("end", "未知状态")),
    ]);

    let mut status_ids = std::collections::HashMap::new();
    for (notice, (flag, comments)) in &status_map {
        let id = ensure_status(pool, notice, flag, comments).await?;
        status_ids.insert(*notice, id);
    }

    let mut inserted = 0usize;
    for env in all_seed_environments() {
        let exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM isahl."zc_id_prot-env_config"
                   WHERE code = $1 AND deleted_at IS NULL
               )"#,
        )
        .bind(env.host)
        .fetch_one(pool)
        .await
        .map_err(AliothError::from)?;

        if exists {
            continue;
        }

        let settings = serde_json::json!({
            "os": env.os,
            "runtime": env.runtime,
            "type": env.type_,
            "services": env.services,
            "uptime": env.uptime,
        });

        let status_id = status_ids
            .get(env.status)
            .copied()
            .expect("status must be seeded");

        let mut tx = pool.begin().await.map_err(AliothError::from)?;

        let env_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_prot-env_config"
               (notice, code, comments, settings, created_by_id)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id"#,
        )
        .bind(env.name)
        .bind(env.host)
        .bind(env.comments)
        .bind(&settings)
        .bind(SEED_USER_ID)
        .fetch_one(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        sqlx::query(
            r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (ref_left, ref_right, created_by_id)
               VALUES ($1, $2, $3)"#,
        )
        .bind(env_id)
        .bind(status_id)
        .bind(SEED_USER_ID)
        .execute(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        tx.commit().await.map_err(AliothError::from)?;
        inserted += 1;
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::testing::{connect_test_db, setup_test_schema_light};

    #[tokio::test]
    async fn test_seed_environments_idempotent() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();

        let first = seed_environments(&pool).await.unwrap();
        let second = seed_environments(&pool).await.unwrap();

        assert_eq!(first, 5, "should insert 5 environments on first run");
        assert_eq!(second, 0, "should be idempotent on second run");

        let count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM isahl."zc_id_prot-env_config" WHERE deleted_at IS NULL"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 5);

        let rel_count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM isahl."zc_id_lifecycle_r_primary-status" WHERE deleted_at IS NULL"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rel_count, 5);
    }
}
