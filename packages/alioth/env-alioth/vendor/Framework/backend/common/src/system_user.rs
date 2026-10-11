//! 系统身份（SYSTEM_USER_ID=1）— auth_users 记录 + NGAC 授权惰性 ensure
//!
//! 系统自动操作（SLA 自动驳回等）以 `SYSTEM_USER_ID` 为审计归属。为保证：
//! 1. 审计 JOIN `auth_users` 可解析（user_type='system'，不可登录）
//! 2. 系统操作经 NGAC 显式授权（admin 属性 × 资源 × 动作）
//! 3. 系统身份创建本身受审计监管
//!
//! `ensure_system_user` 幂等：已存在则跳过，安全重复调用。

use crate::audit::{record_audit_event, Decision};
use crate::SYSTEM_USER_ID;
use sqlx::PgPool;

/// 确保系统用户存在（auth_users + NGAC 授权），幂等。
///
/// 应于系统后台任务（如 SLA 监控）启动时调用一次。
pub async fn ensure_system_user(pool: &PgPool) -> Result<(), sqlx::Error> {
    // 1. auth_users 记录（不可登录：user_type='system'，无密码）
    // 自愈语义：共享测试库/历史数据可能残留 id=1 'standard' 行（测试夹具
    // 先于本 ensure 写入）——ON CONFLICT DO UPDATE 强制归位系统身份，
    // 消除 system_user_test 的顺序依赖（2026-08-26 实证：先跑的二进制
    // 创建 standard id=1 → 断言 user_type='system' 失败）。
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, display_name, user_type, password_hash, is_active, status,
            created_at, updated_at, failed_login_attempts, notification_preferences)
           VALUES ($1, 'system', '系统', 'system', NULL, TRUE, 'active',
                   NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO UPDATE SET
             display_name = '系统',
             user_type = 'system',
             password_hash = NULL,
             is_active = TRUE,
             status = 'active',
             username = 'system'"#,
    )
    .bind(SYSTEM_USER_ID)
    .execute(pool)
    .await?;

    // 1b. 邮箱唯一存储 = auth_user_emails（系统账号审计/通知地址；账号内幂等）
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary)
           SELECT $1, 'system@aliothstudio.local', TRUE
           WHERE NOT EXISTS (
               SELECT 1 FROM isahl_auth.auth_user_emails
               WHERE fk_user = $1 AND email = 'system@aliothstudio.local' AND deleted_at IS NULL)"#,
    )
    .bind(SYSTEM_USER_ID)
    .execute(pool)
    .await?;

    // 2. NGAC 授权：system → admin user_attribute（名取自唯一名字集，不写字面量）
    let admin_attr: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(ngac_host::ADMIN_UA_NAME)
    .fetch_optional(pool)
    .await?
    .unwrap_or_default();

    if admin_attr != 0 {
        sqlx::query(
            r#"INSERT INTO isahl_auth.ngac_user_rr_attribute
               (id, o_name, fk_user, fk_user_attribute, assigned_at, created_at)
               VALUES (isahl.gen_next_zuid(), 'system-admin', $1, $2, NOW(), NOW())
               ON CONFLICT DO NOTHING"#,
        )
        .bind(SYSTEM_USER_ID)
        .bind(admin_attr)
        .execute(pool)
        .await?;
    }

    // 3. 审计：系统身份就绪（决策=permit，记录初始化）
    let _ = record_audit_event(
        pool,
        SYSTEM_USER_ID,
        "system@aliothstudio.local",
        "auth_users/system",
        "system_user.ensure",
        &Decision::Permit,
    )
    .await;

    Ok(())
}
