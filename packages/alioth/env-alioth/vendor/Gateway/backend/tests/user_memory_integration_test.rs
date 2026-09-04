//! UserMemoryStore 集成测试（add-agent-pool-user-memory）。
//!
//! 覆盖：load/save 持久化、per-user 隔离、version 递增。
//! 使用 aliothstudio_test 数据库，负 ID 用户自清。

use ::common::testing::connect_test_db;
use alioth_gateway::api::chat_sessions::memory_store::UserMemoryStore;
use serde_json::json;

const USER_A: i64 = -920001;
const USER_B: i64 = -920002;

async fn cleanup(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM isahl_auth.gateway_user_memory WHERE user_id = ANY($1)")
        .bind(&vec![USER_A, USER_B][..])
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn load_default_empty() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let store = UserMemoryStore::new(pool.clone());
    let memory = store.load(USER_A).await.expect("load");
    assert_eq!(memory, json!({}), "无记录应返回空对象");

    cleanup(&pool).await;
}

#[tokio::test]
async fn save_then_load_roundtrip() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let store = UserMemoryStore::new(pool.clone());
    store
        .save(USER_A, json!({"偏好": "简约风格", "职位": "工程师"}))
        .await
        .expect("save");

    let memory = store.load(USER_A).await.expect("load");
    assert_eq!(
        memory["偏好"].as_str(),
        Some("简约风格"),
        "roundtrip 应保留内容"
    );

    cleanup(&pool).await;
}

#[tokio::test]
async fn per_user_isolated() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let store = UserMemoryStore::new(pool.clone());
    store
        .save(USER_A, json!({"偏好": "A 的偏好"}))
        .await
        .expect("save A");
    store
        .save(USER_B, json!({"偏好": "B 的偏好"}))
        .await
        .expect("save B");

    let mem_a = store.load(USER_A).await.expect("load A");
    let mem_b = store.load(USER_B).await.expect("load B");
    assert_eq!(mem_a["偏好"].as_str(), Some("A 的偏好"));
    assert_eq!(mem_b["偏好"].as_str(), Some("B 的偏好"));
    assert_ne!(mem_a, mem_b, "用户 memory 必须隔离");

    cleanup(&pool).await;
}

#[tokio::test]
async fn save_increments_version() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let store = UserMemoryStore::new(pool.clone());
    store
        .save(USER_A, json!({"v": 1}))
        .await
        .expect("first save");
    store
        .save(USER_A, json!({"v": 2}))
        .await
        .expect("second save");

    let version: i64 =
        sqlx::query_scalar("SELECT version FROM isahl_auth.gateway_user_memory WHERE user_id = $1")
            .bind(USER_A)
            .fetch_one(&pool)
            .await
            .expect("version");
    assert_eq!(version, 2, "第二次 save 应 version 递增");

    cleanup(&pool).await;
}
