//! App 可见性自愈（跨 namespace 通用机制，generalize-app-visibility-seed）
//!
//! 边界（对齐 ngac_seed §7.3 三层种子边界）：本模块只持有**跨 namespace 通用结构**——
//! `resource_type='app'` 的 OA 与 admin UA → read 关联；app_code 全部来自发现面
//! （`apps::read_apps_data`，`Pre-Proc/{ns}/Apps/*/app.json` 或 DEPLOY_PATH apps.json），
//! **零 namespace 业务资源名硬编码**。禁止 UPDATE/DELETE——仅补缺失行（幂等）。
//!
//! 语义（add-app-visibility-ngac-isolation fail-closed 承载物）：任意 ns 首次启动即得
//! 管理面可见（admin → 全部发现 App），新增 App 落盘即自动纳入；非 admin 角色可见性由
//! ns seed 套件（角色策略覆盖）或运行时指派（如 OA 认证效应）供给。

use sqlx::PgPool;

use super::SeedStats;

/// App 可见性自愈入口（`ensure_gateway_seed_self_check` 链，ngac_seed 之后调用——
/// 依赖 policy_class 'default' / admin UA / read access_right 已就绪）。
pub async fn ensure(pool: &PgPool) -> SeedStats {
    let mut stats = SeedStats::default();
    let apps = crate::apps::read_apps_data();
    if apps.is_empty() {
        // 空发现面合法（无 Apps 目录的 ns）；不视为失败
        return stats;
    }
    for app in &apps {
        match ensure_app_oa(pool, &app.app_code).await {
            Ok(true) => stats.created += 1,
            Ok(false) => stats.existing += 1,
            Err(e) => {
                common::telemetry::warn!(
                    "seed[app-visibility]: OA 自愈失败 ({}): {e}",
                    app.app_code
                );
                stats.healed += 1;
            }
        }
        match ensure_admin_association(pool, &app.app_code).await {
            Ok(true) => stats.created += 1,
            Ok(false) => stats.existing += 1,
            Err(e) => {
                common::telemetry::warn!(
                    "seed[app-visibility]: admin 关联自愈失败 ({}): {e}",
                    app.app_code
                );
                stats.healed += 1;
            }
        }
    }
    stats
}

/// 单 App OA upsert（判重键 = resource_type + resource_identifier）；返回是否新建。
async fn ensure_app_oa(pool: &PgPool, app_code: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_object_attribute
            (o_name, fk_policy_class, ancestor_ids, children_ids, resource_type, fk_resource, resource_identifier, created_at)
        SELECT 'app:' || $1, pc.id, '{}', '{}', 'app', isahl.gen_next_zuid(), $1, NOW()
        FROM isahl_auth.ngac_policy_class pc
        WHERE pc.o_name = 'default'
          AND NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_object_attribute oa
            WHERE oa.resource_type = 'app' AND oa.resource_identifier = $1 AND oa.deleted_at IS NULL
          )
        "#,
    )
    .bind(app_code)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// admin UA → App OA read 关联（判重键 = ua+oa 对）；返回是否新建。
async fn ensure_admin_association(pool: &PgPool, app_code: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
        SELECT ua.id, oa.id, ARRAY(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read'), oa.fk_policy_class, NOW()
        FROM isahl_auth.ngac_user_attribute ua
        CROSS JOIN isahl_auth.ngac_object_attribute oa
        WHERE ua.o_name = 'admin' AND ua.deleted_at IS NULL
          AND oa.resource_type = 'app' AND oa.resource_identifier = $1 AND oa.deleted_at IS NULL
          AND NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_association a2
            WHERE a2.fk_user_attribute = ua.id AND a2.fk_object_attribute = oa.id AND a2.deleted_at IS NULL
          )
        "#,
    )
    .bind(app_code)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}
