//! 状态桥读写 —— `zc_id_lifecycle_r_primary-status` / `zc_id_lifecycle_r_status` 唯一载体
//!
//! 规约不变量（ALIOTH_ONTOLOGY_SPEC §1.2）：状态不存源表、不入 comments；
//! ref_left=实体行、ref_right=`stus-*` 字典行。
//! 提取自 contract service `transition.rs`（REUSE_FIRST：procure/contract 共用，
//! 禁止各 service 手搓重复实现）。

use sqlx::{AssertSqlSafe, Executor, Postgres, Transaction};

use crate::AliothError;

/// 状态字典表白名单（sqlx 0.9 动态表名须 AssertSqlSafe 显式标记；
/// 新增 `stus-*` 族在此登记，未知表 fail-visible）
const STATUS_DICT_WHITELIST: &[&str] = &[
    "zc_id_stus-agreement",
    "zc_id_stus-contract",
    "zc_id_stus-prod-made",
    "zc_id_stus-prod-purchase",
    "zc_id_stus-prod-request",
    "zc_id_stus-prod-sales",
    "zc_id_stus-smt-voucher",
    "zc_id_stus-trade",
];

fn ensure_dict(dict_table: &str) -> Result<(), AliothError> {
    if STATUS_DICT_WHITELIST.contains(&dict_table) {
        Ok(())
    } else {
        Err(AliothError::Internal(format!(
            "未知状态字典表: {dict_table}"
        )))
    }
}

/// 读实体当前状态 code（无桥接行 → None；字典 code 原样返回，不剥前缀）。
///
/// executor 泛型：`&PgPool` 或 `&mut PgConnection` / 事务内连接均可。
pub async fn current_status_opt<'e, E>(
    executor: E,
    entity_id: i64,
    dict_table: &str,
) -> Result<Option<String>, AliothError>
where
    E: Executor<'e, Database = Postgres>,
{
    ensure_dict(dict_table)?;
    let sql = format!(
        r#"SELECT st.code FROM "isahl"."zc_id_lifecycle_r_primary-status" ps
           JOIN "isahl"."{dict_table}" st ON st.id = ps.ref_right AND st.deleted_at IS NULL
           WHERE ps.ref_left = $1 AND ps.deleted_at IS NULL
           ORDER BY ps.id DESC LIMIT 1"#,
    );
    let code: Option<String> = sqlx::query_scalar(AssertSqlSafe(sql))
        .bind(entity_id)
        .fetch_optional(executor)
        .await
        .map_err(AliothError::from_sqlx)?;
    Ok(code)
}

/// 事务内 upsert 主状态桥（isahl 冻结无 DB 唯一约束——原位更新活行，无行则插入；
/// 同 accounts-receivable 写路径范式）
/// 事务内 upsert 续约状态桥（`zc_id_lifecycle_r_status`，code='renewal_status'；
/// 双状态分离（contract-dual-status spec）：续约流转只更新续约状态，不动主状态桥）
pub async fn upsert_renewal_status_tx(
    tx: &mut Transaction<'_, Postgres>,
    entity_id: i64,
    status_id: i64,
    user_id: i64,
) -> Result<(), AliothError> {
    let updated = sqlx::query(
        "UPDATE \"isahl\".\"zc_id_lifecycle_r_status\" SET \
           ref_right = $2, updated_at = NOW(), updated_by_id = $3, \
           deleted_at = NULL, deleted_by_id = NULL \
         WHERE ref_left = $1 AND code = 'renewal_status' AND deleted_at IS NULL",
    )
    .bind(entity_id)
    .bind(status_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(AliothError::from_sqlx)?;
    if updated.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO \"isahl\".\"zc_id_lifecycle_r_status\" \
             (ref_left, ref_right, code, notice, created_by_id) VALUES ($1, $2, 'renewal_status', '续约状态流转', $3)",
        )
        .bind(entity_id)
        .bind(status_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(AliothError::from_sqlx)?;
    }
    Ok(())
}

pub async fn upsert_status_tx(
    tx: &mut Transaction<'_, Postgres>,
    entity_id: i64,
    status_id: i64,
    user_id: i64,
) -> Result<(), AliothError> {
    let updated = sqlx::query(
        "UPDATE \"isahl\".\"zc_id_lifecycle_r_primary-status\" SET \
           ref_right = $2, updated_at = NOW(), updated_by_id = $3, \
           deleted_at = NULL, deleted_by_id = NULL \
         WHERE ref_left = $1 AND deleted_at IS NULL",
    )
    .bind(entity_id)
    .bind(status_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(AliothError::from_sqlx)?;
    if updated.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO \"isahl\".\"zc_id_lifecycle_r_primary-status\" \
             (ref_left, ref_right, notice, created_by_id) VALUES ($1, $2, '状态流转', $3)",
        )
        .bind(entity_id)
        .bind(status_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(AliothError::from_sqlx)?;
    }
    Ok(())
}
