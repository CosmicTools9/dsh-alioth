//! 状态桥读写 —— `zc_id_lifecycle_r_primary-status` / `zc_id_lifecycle_r_status` 唯一载体
//!
//! 规约不变量（ALIOTH_ONTOLOGY_SPEC §1.2）：状态不存源表、不入 comments；
//! ref_left=实体行、ref_right=`stus-*` 字典行。
//! 提取自 contract service `transition.rs`（REUSE_FIRST：procure/contract 共用，
//! 禁止各 service 手搓重复实现）。

use sqlx::{Executor, Postgres, Transaction};

use crate::AliothError;

/// 状态字典表 → 静态 SQL：每条 SQL 在**编译期**由 `concat!` 固化（表名是字面量，
/// 运行期无拼串、无 `AssertSqlSafe`）。SQL 正文单一来源（宏内一处），
/// 族成员新增 = 在调用处加一行表名，未知表 fail-visible（不回落、不静默）。
macro_rules! status_dict_sqls {
    ($($table:literal),+ $(,)?) => {
        &[$((
            $table,
            concat!(
                "SELECT st.code FROM \"isahl\".\"zc_id_lifecycle_r_primary-status\" ps \
                 JOIN \"isahl\".\"", $table, "\" st ON st.id = ps.ref_right AND st.deleted_at IS NULL \
                 WHERE ps.ref_left = $1 AND ps.deleted_at IS NULL \
                 ORDER BY ps.id DESC LIMIT 1"
            ),
        )),+]
    };
}

const STATUS_DICT_SQL: &[(&str, &str)] = status_dict_sqls![
    "zc_id_stus-agreement",
    "zc_id_stus-contract",
    "zc_id_stus-prod-made",
    "zc_id_stus-prod-purchase",
    "zc_id_stus-prod-request",
    "zc_id_stus-prod-sales",
    "zc_id_stus-smt-voucher",
    "zc_id_stus-trade",
];

fn status_dict_sql(dict_table: &str) -> Result<&'static str, AliothError> {
    STATUS_DICT_SQL
        .iter()
        .find(|(t, _)| *t == dict_table)
        .map(|(_, sql)| *sql)
        .ok_or_else(|| AliothError::Internal(format!("未知状态字典表: {dict_table}")))
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
    let sql = status_dict_sql(dict_table)?;
    let code: Option<String> = sqlx::query_scalar(sql)
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
            "INSERT INTO \"isahl\".\"zc_id_lifecycle_r_primary-status\" \
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

/// 状态字典行的**阶段归类**（`isahl.status_flag`）
///
/// DDL 约束：该列为 `isahl.status_flag` 枚举、**NOT NULL 且无默认值**（`ENVIRONMENT_SPEC.md §11.4`），
/// 故字典行 INSERT MUST 显式给值。语义 = 「该状态处于本体生命周期的哪个阶段」
/// （`ALIOTH_ONTOLOGY_SPEC.md §4.2`）：
///
/// - `start` 起始态：草稿 / 待受理 / 未激活类
/// - `end` 终态：完成 / 驳回 / 终止类
/// - `doing` 中间态：其余（"已提交 / 审核中 / 已发布 / 已签收"等仍在流转者归此）
///
/// 匹配口径 = **code 词尾段**（`ST-DRAFT` / `cert-state-draft` / `ps-planned` 均命中 `draft`）。
/// 适用面：运行期由调用方提供 code、且拿不到字典行语义上下文（notice / 所属表）的写入路径；
/// 种子与领域字典的逐行取值仍以各自显式声明为准（`Framework/seed/*` 写 `flag` 字面量）。
pub fn flag_for_status_code(code: &str) -> &'static str {
    const END: &[&str] = &[
        "approved",
        "rejected",
        "refused",
        "passed",
        "completed",
        "complete",
        "done",
        "closed",
        "canceled",
        "cancelled",
        "settled",
        "voided",
        "void",
        "expired",
        "invalidated",
        "revoked",
        "withdrawn",
        "retired",
        "scrapped",
        "abolished",
        "obsolete",
        "archived",
        "terminated",
        "disabled",
        "deprecated",
        "failed",
        "ended",
        "finished",
        "compliant",
        // 终态语义补充（与 Framework/seed 的显式取值对齐：放行/修复/删除/处理/实施/适用/执行/逾期）
        "released",
        "fixed",
        "deleted",
        "handled",
        "implemented",
        "applicable",
        "executed",
        "overdue",
        "resolved",
        "resigned",
    ];
    const START: &[&str] = &[
        "draft", "new", "pending", "open", "created", "init", "unread", "inactive", "planned",
        "applied", "todo",
    ];
    let trimmed = code.trim();
    let full = trimmed.to_ascii_lowercase();
    let tail = trimmed
        .rsplit(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if END.contains(&full.as_str()) || END.contains(&tail.as_str()) {
        return "end";
    }
    if START.contains(&full.as_str()) || START.contains(&tail.as_str()) {
        return "start";
    }
    "doing"
}

pub async fn upsert_status_tx(
    tx: &mut Transaction<'_, Postgres>,
    entity_id: i64,
    status_id: i64,
    user_id: i64,
) -> Result<(), AliothError> {
    // 模型约束：`zc_id_lifecycle_r_primary-status` 上 `UNIQUE (ref_left)` 覆盖**含软删行**，
    // 故按 (ref_left) 原地复活/更新，而不是「看不到未删行就 INSERT」（否则撞唯一键）。
    let updated = sqlx::query(
        "UPDATE \"isahl\".\"zc_id_lifecycle_r_primary-status\" SET \
           ref_right = $2, updated_at = NOW(), updated_by_id = $3, \
           deleted_at = NULL, deleted_by_id = NULL \
         WHERE ref_left = $1",
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
             (ref_left, ref_right, notice, created_by_id) VALUES ($1, $2, '状态流转', $3) \
             ON CONFLICT (ref_left) DO UPDATE SET ref_right = EXCLUDED.ref_right, \
               updated_at = NOW(), deleted_at = NULL, deleted_by_id = NULL",
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

#[cfg(test)]
mod tests {
    use super::flag_for_status_code;

    #[test]
    fn flag_for_status_code_阶段归类() {
        // 起始态
        for c in [
            "draft",
            "ST-DRAFT",
            "cert-state-draft",
            "ps-planned",
            "pending",
            "unread",
            "IOT-INACTIVE",
        ] {
            assert_eq!(flag_for_status_code(c), "start", "期望 start: {c}");
        }
        // 终态
        for c in [
            "approved",
            "rejected",
            "completed",
            "CNT-DONE",
            "closed",
            "ST-SETTLED",
            "PTC-TERMINATED",
            "CERT-REVOKED",
            "sign-released",
            "invalidated",
            "CT-INVALIDATED",
        ] {
            assert_eq!(flag_for_status_code(c), "end", "期望 end: {c}");
        }
        // 中间态（仍在流转：已提交 / 审核中 / 已发布 / 已签收 / 已读）
        for c in [
            "read",
            "published",
            "cb-effective",
            "ag-submitted",
            "cert-state-review",
            "ST-SIGNED",
            "ra-assessing",
            "",
        ] {
            assert_eq!(flag_for_status_code(c), "doing", "期望 doing: {c}");
        }
    }
}
