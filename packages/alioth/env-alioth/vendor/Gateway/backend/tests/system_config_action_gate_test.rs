//! 系统设置动作 admin 门控集成测试
//! （add-system-config-ai-action 2.5：非管理员被拒 + 零写库 + 零审计）。
//!
//! 通道：`GatewayActionHandler::run_action("system_config_apply")` 的执行体
//! （`adapters/system_config_action::apply`）——admin 判据 = `common::permissions::is_admin_user`
//! （与 require_admin 同一实现）。非管理员（无任何 NGAC UA 指派）⇒ `ADMIN_REQUIRED`，
//! `isahl.zc_id_prot-llm_config` 零变更、`isahl_audit.audit_events` 零 system_config.* 事件。
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
    let username = format!("rtnsyscfg-gate-{nanos}");
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

    let code = format!("{username}-cfg");
    let params = json!({
        "operations": [{
            "op": "create_config", "category": "llm", "provider": "deepseek",
            "code": code, "notice": "不应落库",
            "settings": { "model": "deepseek-v4" }
        }]
    });
    let err = alioth_gateway::api::chat_sessions::adapters::system_config_action::apply(
        &pool, uid, &params,
    )
    .await
    .expect_err("非管理员必须被拒");
    assert!(err.contains("ADMIN_REQUIRED"), "{err}");

    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM isahl.\"zc_id_prot-llm_config\" WHERE code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0, "非管理员 MUST NOT 产生任何写入");

    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_audit.audit_events \
         WHERE user_id = $1 AND operation LIKE 'system_config.%'",
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 0, "被拒路径 MUST NOT 留 system_config.* 审计");

    // 自清
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await;
}
