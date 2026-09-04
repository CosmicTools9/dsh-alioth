//! 状态因子种子数据
//!
//! 为 system-settings 模块提供通用状态记录（healthy / warning / unknown）。
//! 幂等执行：按 notice 去重，已存在则跳过。

use common::error::AliothError;
use sqlx::PgPool;

const SEED_USER_ID: i64 = 1;

/// 单个待插入的状态种子。
#[derive(Debug, Clone)]
struct SeedStatus {
    notice: &'static str,
    flag: &'static str,
    comments: &'static str,
}

fn all_seed_statuses() -> Vec<SeedStatus> {
    vec![
        SeedStatus {
            notice: "healthy",
            flag: "start",
            comments: "正常运行状态",
        },
        SeedStatus {
            notice: "warning",
            flag: "doing",
            comments: "需要关注状态",
        },
        SeedStatus {
            notice: "unknown",
            flag: "end",
            comments: "未知状态",
        },
    ]
}

/// 向数据库预置通用状态记录。
///
/// 按 `notice` 去重；已存在非删除记录则跳过，保证幂等。
/// 返回 (notice -> id) 映射。
pub async fn seed_statuses(
    pool: &PgPool,
) -> Result<std::collections::HashMap<String, i64>, AliothError> {
    let seeds = all_seed_statuses();
    let mut ids = std::collections::HashMap::with_capacity(seeds.len());

    for seed in seeds {
        let id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM isahl.zc_id_status
               WHERE notice = $1 AND deleted_at IS NULL
               LIMIT 1"#,
        )
        .bind(seed.notice)
        .fetch_optional(pool)
        .await
        .map_err(AliothError::from)?;

        let id = match id {
            Some(existing) => existing,
            None => sqlx::query_scalar(
                r#"INSERT INTO isahl.zc_id_status (notice, flag, comments, created_by_id)
                       VALUES ($1, $2::isahl.status_flag, $3, $4)
                       RETURNING id"#,
            )
            .bind(seed.notice)
            .bind(seed.flag)
            .bind(seed.comments)
            .bind(SEED_USER_ID)
            .fetch_one(pool)
            .await
            .map_err(AliothError::from)?,
        };
        ids.insert(seed.notice.to_string(), id);
    }

    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::testing::{connect_test_db, setup_test_schema_light};

    #[tokio::test]
    async fn test_seed_statuses_idempotent() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();

        let first = seed_statuses(&pool).await.unwrap();
        let second = seed_statuses(&pool).await.unwrap();

        assert_eq!(first.len(), 3, "should insert 3 statuses on first run");
        assert_eq!(
            second.len(),
            3,
            "should return same 3 statuses on second run"
        );

        for (notice, id1) in &first {
            let id2 = second.get(notice).expect("same notice");
            assert_eq!(id1, id2, "idempotent ids must match");
        }
    }
}
