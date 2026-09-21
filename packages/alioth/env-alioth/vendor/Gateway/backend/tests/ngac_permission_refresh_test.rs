//! 会话内权限变更与 fail-closed（refactor-chat-ai-subject-identity-memory task 9.2）
//!
//! 覆盖 chat 侧的权限契约（裁决本身由 PEP/PDP 每请求承担，不在本测试范围）：
//! - **逐轮解析**：NGAC 属性分配在会话进行中变更后，下一轮解析结果即反映
//!   （不缓存陈旧快照；`orchestrator.rs` 每轮调用本函数）
//! - **fail-closed**：未登记用户/解析失败 ⇒ `Err`（调用方不注入权限段，
//!   MUST NOT 回落陈旧快照）
//!
//! fixture 落在 `isahl_auth`（工程 schema，非冻结）负 ID 自建自清。

use ::common::testing::connect_test_db;
use sqlx::PgPool;

const TEST_USER: i64 = -940001;
const TEST_POLICY_CLASS: i64 = -940101;
const TEST_ATTRIBUTE: i64 = -940201;
const TEST_ASSIGNMENT: i64 = -940301;

/// 自清 fixture（属性关系 → 属性 → 策略类 → 用户）。
async fn cleanup(pool: &PgPool) {
    let _ = sqlx::query(r#"DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE id = $1"#)
        .bind(TEST_ASSIGNMENT)
        .execute(pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1"#)
        .bind(TEST_USER)
        .execute(pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl_auth.ngac_user_attribute WHERE id = $1"#)
        .bind(TEST_ATTRIBUTE)
        .execute(pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl_auth.ngac_policy_class WHERE id = $1"#)
        .bind(TEST_POLICY_CLASS)
        .execute(pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl_auth.auth_users WHERE id = $1"#)
        .bind(TEST_USER)
        .execute(pool)
        .await;
}

async fn insert_user(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, name, username, email, password_hash)
           VALUES ($1, 'task-9.2', 'task-9.2', 'task-9.2@test.local', 'x')"#,
    )
    .bind(TEST_USER)
    .execute(pool)
    .await
    .expect("insert auth user");
}

#[tokio::test]
async fn permissions_re_resolved_per_turn_and_fail_closed() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    // 未登记用户 → Err（调用方 fail-closed：不注入权限段，不回落陈旧快照）
    assert!(
        alioth_gateway::ngac::resolve_user_permissions(&pool, TEST_USER)
            .await
            .is_err(),
        "未登记用户 MUST 解析失败（fail-closed）"
    );

    insert_user(&pool).await;
    let before = alioth_gateway::ngac::resolve_user_permissions(&pool, TEST_USER)
        .await
        .expect("已登记用户可解析");
    assert_eq!(
        before["userAttributes"].as_array().map(Vec::len),
        Some(0),
        "初始无 NGAC 属性"
    );

    // 会话进行中授予一条 NGAC 属性（模拟权限变更）
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_policy_class (id, o_name) VALUES ($1, 'pc-task-9.2')"#,
    )
    .bind(TEST_POLICY_CLASS)
    .execute(&pool)
    .await
    .expect("insert policy class");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (id, o_name, fk_policy_class)
           VALUES ($1, 'view:task-9.2', $2)"#,
    )
    .bind(TEST_ATTRIBUTE)
    .bind(TEST_POLICY_CLASS)
    .execute(&pool)
    .await
    .expect("insert user attribute");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (id, fk_user, fk_user_attribute)
           VALUES ($1, $2, $3)"#,
    )
    .bind(TEST_ASSIGNMENT)
    .bind(TEST_USER)
    .bind(TEST_ATTRIBUTE)
    .execute(&pool)
    .await
    .expect("assign attribute");

    // 下一轮解析 MUST 反映新授予（逐轮解析，非会话级缓存）
    let after = alioth_gateway::ngac::resolve_user_permissions(&pool, TEST_USER)
        .await
        .expect("第二轮解析");
    let attributes: Vec<String> = after["userAttributes"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        attributes.iter().any(|attr| attr == "view:task-9.2"),
        "会话内新授予 MUST 在下一轮解析生效: {attributes:?}"
    );

    cleanup(&pool).await;
}
