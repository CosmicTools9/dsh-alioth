//! 版本通用种子数据
//!
//! 为 `isahl.zc_id_version` 预置演示版本链。
//! 幂等执行：按 (tpl_id, version_number) 去重，已存在非删除记录则跳过。

use chrono::Utc;
use common::error::AliothError;
use sqlx::PgPool;

const SEED_USER_ID: i64 = 1;

#[derive(Debug, Clone)]
struct SeedVersion {
    tpl_id: i64,
    version_number: i64,
    revision: i64,
}

fn all_seed_versions() -> Vec<SeedVersion> {
    vec![
        // 模板/实体 1 的版本链
        SeedVersion {
            tpl_id: 1,
            version_number: 1,
            revision: 0,
        },
        SeedVersion {
            tpl_id: 1,
            version_number: 2,
            revision: 0,
        },
        SeedVersion {
            tpl_id: 1,
            version_number: 3,
            revision: 1,
        },
        // 模板/实体 2 的版本链
        SeedVersion {
            tpl_id: 2,
            version_number: 1,
            revision: 0,
        },
        SeedVersion {
            tpl_id: 2,
            version_number: 2,
            revision: 0,
        },
    ]
}

/// 向数据库预置通用版本链记录。
///
/// 幂等：同一 `tpl_id` + `version_number` 已存在非删除记录则跳过。
/// 按 version_number 升序插入并自动维护 `fk_previous` 链。
pub async fn seed_versions(pool: &PgPool) -> Result<usize, AliothError> {
    let mut inserted = 0usize;

    for v in all_seed_versions() {
        let exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM isahl.zc_id_version
                   WHERE tpl_id = $1
                     AND tk_version = $2
                     AND deleted_at IS NULL
               )"#,
        )
        .bind(v.tpl_id)
        .bind(v.version_number)
        .fetch_one(pool)
        .await
        .map_err(AliothError::from)?;
        if exists {
            continue;
        }

        // 新记录 fk_previous 为 NULL，成为新的链头。
        let new_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl.zc_id_version
               (tpl_id, tk_version, reversion, created_by_id, created_at)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id"#,
        )
        .bind(v.tpl_id)
        .bind(v.version_number)
        .bind(v.revision)
        .bind(SEED_USER_ID)
        .bind(Utc::now())
        .fetch_one(pool)
        .await
        .map_err(AliothError::from)?;

        // 旧链头（同一 tpl_id，fk_previous IS NULL，id != new_id）指向新记录。
        sqlx::query(
            r#"UPDATE isahl.zc_id_version
               SET fk_previous = $1, updated_at = NOW()
               WHERE tpl_id = $2
                 AND id != $1
                 AND fk_previous IS NULL
                 AND deleted_at IS NULL"#,
        )
        .bind(new_id)
        .bind(v.tpl_id)
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
    async fn seed_versions_is_idempotent() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();
        // 清理版本表（测试库共享，幂等断言依赖空起点）
        sqlx::query("DELETE FROM isahl.zc_id_version")
            .execute(&pool)
            .await
            .unwrap();
        let first = seed_versions(&pool)
            .await
            .expect("first seed should succeed");
        let second = seed_versions(&pool)
            .await
            .expect("second seed should succeed");

        assert_eq!(second, 0, "re-seeding should be idempotent");
        assert!(first >= 5, "should seed at least 5 versions");
    }
}
