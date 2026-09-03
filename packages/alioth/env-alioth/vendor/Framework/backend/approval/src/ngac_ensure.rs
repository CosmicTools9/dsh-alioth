//! NGAC employee UA 幂等确保（跨 namespace 通用，add-gateway-seed-self-heal 收敛）
//!
//! 注册审批通过后授予默认 employee UA：查找或创建（gen_next_zuid + 首个 policy class），
//! 再经 ngac_user_rr_attribute 幂等指派。employee UA 无祖先（空层级），
//! 不触碰层级写一致性校验义务（NGAC_SPEC §4.1b）。
//!
//! 收敛点：Gateway `api/approvals.rs` 与本 crate `handlers/approve_reject.rs`
//! 原各持一份实现——统一为本函数；创建分支保留为兜底（seed 组件启动前审批的极端时序）。

use sqlx::PgPool;

/// 为指定用户幂等指派 employee UA。返回 UA id（任何一步失败 → None + warn）。
pub async fn ensure_employee_ua(pool: &PgPool, user_id: i64) -> Option<i64> {
    let policy_class: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()?;

    let ua_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl_auth.ngac_user_attribute
           WHERE o_name = 'employee' AND deleted_at IS NULL
             AND fk_policy_class = $1 LIMIT 1"#,
    )
    .bind(policy_class)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // seed 组件已预置 employee UA；此处保留创建兜底（防 seed 启动前审批的时序窗口）
    let ua_id = match ua_id {
        Some(id) => id,
        None => match sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl_auth.ngac_user_attribute
               (id, o_name, fk_policy_class, ancestor_ids, children_ids)
               VALUES (isahl.gen_next_zuid(), 'employee', $1, '{}'::bigint[], '{}'::bigint[])
               RETURNING id"#,
        )
        .bind(policy_class)
        .fetch_one(pool)
        .await
        {
            Ok(id) => id,
            Err(e) => {
                common::telemetry::warn!("ensure_employee_ua: 创建 employee UA 失败: {e}");
                return None;
            }
        },
    };

    if let Err(e) = sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, o_name)
           VALUES ($1, $2, 'employee')
           ON CONFLICT (fk_user, fk_user_attribute)
           DO UPDATE SET deleted_at = NULL, updated_at = NOW()"#,
    )
    .bind(user_id)
    .bind(ua_id)
    .execute(pool)
    .await
    {
        common::telemetry::warn!("ensure_employee_ua: user {user_id} employee UA 指派失败: {e}");
        return None;
    }

    Some(ua_id)
}
