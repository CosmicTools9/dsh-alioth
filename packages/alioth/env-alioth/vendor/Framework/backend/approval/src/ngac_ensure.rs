//! NGAC employee UA 幂等确保（跨 namespace 通用，add-gateway-seed-self-heal 收敛）
//!
//! 注册审批通过后授予默认 employee UA：查找或创建（gen_next_zuid + 首个 policy class），
//! 再经 ngac_user_rr_attribute 幂等指派。employee UA 无祖先（空层级），
//! 不触碰层级写一致性校验义务（NGAC_SPEC §4.1b）。
//!
//! 收敛点：Gateway `api/approvals.rs` 与本 crate `handlers/approve_reject.rs`
//! 原各持一份实现——统一为本函数；创建分支保留为兜底（seed 组件启动前审批的极端时序）。
//!
//! Phase C（G6 收口）：enterprise UA 增量同步与 employee UA 指派同收口在本文件——
//! 指派成功后内置调用 [`sync_enterprise_from_employee`]，使「注册审批授予 employee UA」
//! 这一唯一授予路径（Gateway approvals.rs / approve_reject.rs 已收敛至此，无并行
//! 重复分支）自动获得 enterprise 增量同步且不会双调；Gateway entity_binding 绑定时刻
//! 的静态复制亦改为调同一 helper（见该处注释）。

use sqlx::PgPool;

/// 为指定用户幂等指派 employee UA。返回 UA id（任何一步失败 → None + warn）。
/// 指派成功后内置增量同步该用户 enterprise UA 授权（Phase C G6，见模块头）；
/// 同步失败仅 warn 不阻断返回（helper 内部容错）。
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

    // Phase C（G6）：指派成功后增量同步该用户 enterprise UA 授权——employee 此后
    // 新增授权无需回拷静态复制（原绑定时刻一次性复制已废除）。收口说明：本函数是
    // employee UA 授予的唯一收敛点（Gateway approvals.rs / approve_reject.rs 历史
    // 实现已统一至此），同步内置于此即覆盖全部授予路径且无双调风险；helper 内部
    // 容错（失败 warn 返回 0），此处不阻断返回。
    let synced = sync_enterprise_from_employee(pool, user_id).await;
    if synced > 0 {
        common::telemetry::info!(
            "ensure_employee_ua: user {user_id} enterprise UA 增量同步 {synced} 条授权"
        );
    }

    Some(ua_id)
}

/// Phase C（G6）：把该用户 employee UA 的当前全部 live association 增量复制到该用户
/// 全部活跃 enterprise UA（ngac_user_rr_attribute 活行 join UA o_name='enterprise'）。
/// 目标缺失即补、已存在即跳过（NOT EXISTS 幂等）——employee 此后新增授权经本 helper
/// 增量同步，不再依赖绑定时刻的静态快照复制。
///
/// 调用前提：enterprise UA 已幂等指派（helper 经 rr 活行定位目标 UA，须先指派后调用）。
/// 容错：查询失败仅 warn 并返回 0，不向调用方抛错（增强投递不阻断主流程，对齐
/// ensure_employee_ua 语义）。返回本次实际复制条数。
pub async fn sync_enterprise_from_employee(pool: &PgPool, user_id: i64) -> i64 {
    let res = sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_association
           (id, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class)
           SELECT isahl.gen_next_zuid(), c.fk_user_attribute, c.fk_object_attribute,
                  c.ak_access_rights, c.fk_policy_class
           FROM (
             SELECT DISTINCT ent_rr.fk_user_attribute, a.fk_object_attribute,
                             a.ak_access_rights, a.fk_policy_class
             FROM isahl_auth.ngac_user_rr_attribute emp_rr
             JOIN isahl_auth.ngac_user_attribute emp_ua
               ON emp_ua.id = emp_rr.fk_user_attribute
              AND emp_ua.o_name = 'employee' AND emp_ua.deleted_at IS NULL
             JOIN isahl_auth.ngac_association a
               ON a.fk_user_attribute = emp_ua.id AND a.deleted_at IS NULL
             JOIN isahl_auth.ngac_user_rr_attribute ent_rr
               ON ent_rr.fk_user = emp_rr.fk_user AND ent_rr.deleted_at IS NULL
             JOIN isahl_auth.ngac_user_attribute ent_ua
               ON ent_ua.id = ent_rr.fk_user_attribute
              AND ent_ua.o_name = 'enterprise' AND ent_ua.deleted_at IS NULL
             WHERE emp_rr.fk_user = $1 AND emp_rr.deleted_at IS NULL
           ) c
           WHERE NOT EXISTS (
             SELECT 1 FROM isahl_auth.ngac_association x
             WHERE x.fk_user_attribute = c.fk_user_attribute
               AND x.fk_object_attribute = c.fk_object_attribute
               AND x.deleted_at IS NULL
           )"#,
    )
    .bind(user_id)
    .execute(pool)
    .await;

    match res {
        Ok(r) => r.rows_affected() as i64,
        Err(e) => {
            common::telemetry::warn!(
                "sync_enterprise_from_employee: user {user_id} enterprise UA 增量同步失败: {e}"
            );
            0
        }
    }
}
