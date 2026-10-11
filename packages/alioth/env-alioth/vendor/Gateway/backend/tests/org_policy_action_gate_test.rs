#![cfg(feature = "sso")]

//! 权限规范动作 admin 门控集成测试
//! （route-ngac-policy-authoring-through-ai-assist 2.7：非管理员被拒 + 零写库）。
//!
//! 通道：`GatewayActionHandler::run_action("ngac_org_policy_apply")` 的执行体
//! （`adapters/org_policy_action::apply`）——admin 判据 = `common::permissions::is_admin_user`
//! （与 require_admin 同一实现）。非管理员（无任何 NGAC UA 指派）⇒ `ADMIN_REQUIRED`，
//! `org_policy_label` 零变更。
//!
//! 使用 aliothstudio_test 库；自建用户自清。

use ::common::testing::connect_test_db;
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    connect_test_db().await
}

#[tokio::test]
async fn non_admin_rejected_and_zero_write() {
    let pool = connect().await;
    gateway_sso::facade::ngac::ensure::ensure_ngac_extension_tables(&pool).await;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let username = format!("rtngac-gate-{nanos}");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, is_active, created_at, updated_at) \
         VALUES (isahl.gen_next_zuid(), $1, 'active', true, NOW(), NOW())",
    )
    .bind(&username)
    .execute(&pool)
    .await
    .expect("insert user");
    let uid: i64 = sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username = $1")
        .bind(&username)
        .fetch_one(&pool)
        .await
        .expect("fetch user");

    let code = format!("{username}-label");
    let params = json!({
        "operations": [{ "op": "create_label", "code": code, "rank": 1, "notice": "不应落库" }]
    });
    let err =
        alioth_gateway::api::chat_sessions::adapters::org_policy_action::apply(&pool, uid, &params)
            .await
            .expect_err("非管理员必须被拒");
    assert!(err.contains("ADMIN_REQUIRED"), "{err}");

    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM isahl_auth.org_policy_label WHERE code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0, "非管理员 MUST NOT 产生任何写入");

    // 自清
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await;
}
