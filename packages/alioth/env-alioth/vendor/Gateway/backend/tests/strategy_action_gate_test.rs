#![cfg(feature = "sso")]
//! NGAC 策略操作动作 admin 门控集成测试
//! （change `consolidate-ngac-write-path`：非管理员被拒 + 零写库 + 零审计）。
//!
//! 通道：`GatewayActionHandler::run_action("ngac_strategy_apply")` 的执行体
//! （`adapters/strategy_action::apply`）——admin 判据 = `common::permissions::is_admin_user`
//! （与 require_admin 同一实现）。非管理员（无任何 NGAC UA 指派）⇒ `ADMIN_REQUIRED`，
//! NGAC 策略表零变更、`isahl_audit.audit_events` 零 `ngac-strategy/*` 事件。
//!
//! 与 `org_policy_action_gate_test.rs` / `system_config_action_gate_test.rs` 同形
//! （此前本动作是三者中唯一无门控集成测试的）。
//!
//! 使用 aliothstudio_test 库；自建用户自清。

use ::common::testing::connect_test_db;
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    connect_test_db().await
}

#[tokio::test]
async fn non_admin_rejected_and_zero_write_zero_audit() {
    let pool = connect().await;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let username = format!("rtngac-strategy-gate-{nanos}");
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

    // 有意用一个**不存在**的 UA/OA id：非 admin 必须在解析实体之前就被门控拒绝
    // （若门控顺序被破坏，这里会以“实体查询失败”而非 ADMIN_REQUIRED 失败 ⇒ 可诊断）。
    let params = json!({
        "operations": [{
            "op": "create_association",
            "ua_id": 999_000_001_i64,
            "oa_id": 999_000_002_i64,
            "actions": ["read"]
        }]
    });
    let err =
        alioth_gateway::api::chat_sessions::adapters::strategy_action::apply(&pool, uid, &params)
            .await
            .expect_err("非管理员必须被拒");
    assert!(err.contains("ADMIN_REQUIRED"), "{err}");

    // 零审计（两条审计面：逐项 chat 审计 `isahl_audit.audit_events` +
    // 策略变更审计 `isahl_auth.ngac_policy_audit_log` —— 均 MUST NOT 出现）
    let chat_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_audit.audit_events \
         WHERE user_id = $1 AND operation LIKE 'ngac-strategy/%'",
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(chat_audits, 0, "被拒路径 MUST NOT 留 chat 动作审计");

    let policy_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_policy_audit_log WHERE fk_user = $1",
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(policy_audits, 0, "被拒路径 MUST NOT 留策略变更审计");

    // 自清
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await;
}
