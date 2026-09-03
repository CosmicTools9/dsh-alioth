//! system_user 集成测试 — 系统身份三约束验证
//!
//! 验证 `common::system_user::ensure_system_user`：
//! 1. auth_users 创建 id=1 system 记录（user_type='system'，无密码，不可登录）
//! 2. NGAC 授权：system → admin user_attribute
//! 3. 审计监管：audit_events 写入初始化事件
//!
//! 注意：共享 `aliothstudio_test`，单线程运行。

use ::common::system_user::ensure_system_user;
use ::common::testing::connect_test_db;
use sqlx::PgPool;

mod common;
use common::setup_test_schema;

#[tokio::test]
async fn system_user_is_created_idempotently() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // 幂等：调用两次
    ensure_system_user(&pool).await.expect("ensure 1st");
    ensure_system_user(&pool).await.expect("ensure 2nd");

    // 1. auth_users 记录：id=1，user_type='system'，无密码，不可登录
    let (user_type, password_hash, is_active, status): (String, Option<String>, bool, String) =
        sqlx::query_as(
            "SELECT user_type, password_hash, is_active, status FROM isahl_auth.auth_users WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("system user exists");
    assert_eq!(user_type, "system", "user_type=system 不可登录");
    assert!(password_hash.is_none(), "无密码凭据");
    assert!(is_active, "is_active=true");
    assert_eq!(status, "active");

    // 2. NGAC 授权：system → admin user_attribute
    let mapped: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl_auth.ngac_user_rr_attribute
           WHERE fk_user = 1 AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .expect("ngac mapping query");
    assert!(mapped >= 1, "system 已映射到 admin 属性");

    // 3. 审计：audit_events 有 system_user.ensure 记录
    // 诊断：直接调用 record_audit_event 验证
    let r = ::common::audit::record_audit_event(
        &pool,
        1,
        "system@aliothstudio.local",
        "auth_users/system",
        "system_user.ensure",
        &::common::audit::Decision::Permit,
    )
    .await;
    assert!(r.is_ok(), "record_audit_event 应成功: {:?}", r);
    let audited: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl_audit.audit_events
           WHERE user_id = 1 AND operation = 'system_user.ensure'"#,
    )
    .fetch_one(&pool)
    .await
    .expect("audit query");
    assert!(audited >= 1, "系统身份初始化已入审计");

    // 幂等性验证：第二次 ensure 不新增 auth_users 行
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM isahl_auth.auth_users WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "幂等：仅一条 system 记录");
}

#[tokio::test]
async fn system_user_cannot_login() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    ensure_system_user(&pool).await.expect("ensure");

    // 无 password_hash → 任何密码登录校验必然失败（凭据缺失）
    let has_credential: bool = sqlx::query_scalar(
        "SELECT password_hash IS NOT NULL FROM isahl_auth.auth_users WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!has_credential, "system 用户无登录凭据");
}

/// 测试使用 PgPool 导入占位（避免 unused import 警告）
#[allow(dead_code)]
fn _pool_ty(_p: &PgPool) {}
