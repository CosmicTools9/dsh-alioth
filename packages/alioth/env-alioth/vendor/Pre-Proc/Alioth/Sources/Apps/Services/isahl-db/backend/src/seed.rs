//! 身份实体种子数据
//!
//! 为 system-settings 模块的许可证、环境等提供基础身份引用。
//! 幂等执行：按 code 去重。

use common::error::AliothError;
use sqlx::PgPool;

const SEED_USER_ID: i64 = 1;

#[derive(Debug, Clone)]
struct SeedIdentity {
    name: &'static str,
    code: &'static str,
    notice: &'static str,
}

fn all_seed_identities() -> Vec<SeedIdentity> {
    vec![
        SeedIdentity {
            name: "Alioth 平台",
            code: "alioth-platform",
            notice: "Alioth 平台自身身份",
        },
        SeedIdentity {
            name: "示例供应商",
            code: "demo-vendor",
            notice: "用于许可证与认证的示例供应商",
        },
    ]
}

/// 向数据库预置基础身份记录。
///
/// 按 `code` 去重；已存在非删除记录则跳过，保证幂等。
/// 返回插入数量。
pub async fn seed_identities(pool: &PgPool) -> Result<usize, AliothError> {
    let mut inserted = 0usize;

    for seed in all_seed_identities() {
        let exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM isahl.zc_id_subjects
                   WHERE code = $1 AND deleted_at IS NULL
               )"#,
        )
        .bind(seed.code)
        .fetch_one(pool)
        .await
        .map_err(AliothError::from)?;

        if exists {
            continue;
        }

        sqlx::query(
            r#"INSERT INTO isahl.zc_id_subjects (notice, code, comments, created_by_id)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(seed.name)
        .bind(seed.code)
        .bind(seed.notice)
        .bind(SEED_USER_ID)
        .execute(pool)
        .await
        .map_err(AliothError::from)?;

        inserted += 1;
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::testing::{connect_test_db, setup_test_schema_light};

    #[tokio::test]
    async fn test_seed_identities_idempotent() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();
        sqlx::query("DELETE FROM isahl.zc_id_subjects")
            .execute(&pool)
            .await
            .unwrap();

        let first = seed_identities(&pool).await.unwrap();
        let second = seed_identities(&pool).await.unwrap();

        assert_eq!(first, 2, "should insert 2 identities on first run");
        assert_eq!(second, 0, "should be idempotent on second run");
    }
}
