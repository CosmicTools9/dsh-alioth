//! 双层记忆 store 集成测试（refactor-chat-ai-subject-identity-memory）。
//!
//! 覆盖：L1 主体层 / L2 对话方层 load-save 往返、全量替换（新的覆盖旧的）、
//! version 递增、跨主体与跨对话方隔离、两层互不覆盖。
//! 使用 aliothstudio_test 数据库，负 ID 自清（无 FK，任意 id 可测）。

use ::common::testing::connect_test_db;
use alioth_gateway::api::chat_sessions::memory_store::ChatMemoryStore;
use serde_json::json;

const S_EMPTY: i64 = -930001;
const S_ROUNDTRIP: i64 = -930002;
const S_COUNTERPART: i64 = -930003;
const S_ISOLATION_A: i64 = -930004;
const S_ISOLATION_B: i64 = -930005;
const S_VERSION: i64 = -930006;
const C_EMPTY: i64 = -930101;
const C_ROUNDTRIP: i64 = -930102;
const C_ISOLATION_A1: i64 = -930103;
const C_ISOLATION_A2: i64 = -930104;
const C_VERSION: i64 = -930105;

/// 020 两表幂等 ensure：测试库重置后表会丢失；与 migration 020 同构
/// （isahl_auth 工程 schema ensure 模式，参照 chat_sessions_integration_test 的 019 先例）。
async fn ensure_tables(pool: &sqlx::PgPool) {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.chat_ai_subject_memory (
            subject_id bigint PRIMARY KEY,
            memory     jsonb NOT NULL DEFAULT '{}'::jsonb,
            version    bigint NOT NULL DEFAULT 1,
            updated_at timestamptz NOT NULL DEFAULT now()
        )"#,
    )
    .execute(pool)
    .await
    .expect("ensure chat_ai_subject_memory (020)");
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.chat_ai_counterpart_memory (
            subject_id     bigint NOT NULL,
            counterpart_id bigint NOT NULL,
            memory         jsonb NOT NULL DEFAULT '{}'::jsonb,
            version        bigint NOT NULL DEFAULT 1,
            updated_at     timestamptz NOT NULL DEFAULT now(),
            PRIMARY KEY (subject_id, counterpart_id)
        )"#,
    )
    .execute(pool)
    .await
    .expect("ensure chat_ai_counterpart_memory (020)");
}

/// 各测试仅清理**自己**的 id 空间——测试并行执行，共享 id 的清理会互删。
async fn cleanup(pool: &sqlx::PgPool, subjects: &[i64]) {
    sqlx::query("DELETE FROM isahl_auth.chat_ai_subject_memory WHERE subject_id = ANY($1)")
        .bind(subjects)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.chat_ai_counterpart_memory WHERE subject_id = ANY($1)")
        .bind(subjects)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn load_default_empty_both_layers() {
    let pool = connect_test_db().await;
    ensure_tables(&pool).await;
    cleanup(&pool, &[S_EMPTY]).await;

    let store = ChatMemoryStore::new(pool.clone());
    let layers = store.load(S_EMPTY, Some(C_EMPTY)).await.expect("load");
    assert_eq!(layers.subject, json!({}), "无记录主体层应为空对象");
    assert_eq!(
        layers.counterpart,
        Some(json!({})),
        "无记录对话方层应为空对象"
    );

    let layers = store.load(S_EMPTY, None).await.expect("load");
    assert!(layers.counterpart.is_none(), "无对话方上下文时不加载 L2");

    cleanup(&pool, &[S_EMPTY]).await;
}

#[tokio::test]
async fn subject_layer_roundtrip_and_full_replace() {
    let pool = connect_test_db().await;
    ensure_tables(&pool).await;
    cleanup(&pool, &[S_ROUNDTRIP]).await;

    let store = ChatMemoryStore::new(pool.clone());
    store
        .save_subject(S_ROUNDTRIP, json!({"偏好": "简约风格", "职位": "工程师"}))
        .await
        .expect("save subject");
    let loaded = store.load_subject(S_ROUNDTRIP).await.expect("load subject");
    assert_eq!(loaded.get("偏好").unwrap(), "简约风格");

    // 全量替换：新的覆盖旧的（不合并）
    store
        .save_subject(S_ROUNDTRIP, json!({"偏好": "详实风格"}))
        .await
        .expect("replace subject");
    let loaded = store.load_subject(S_ROUNDTRIP).await.expect("load subject");
    assert_eq!(loaded.get("偏好").unwrap(), "详实风格");
    assert!(loaded.get("职位").is_none(), "全量替换须丢弃旧键");

    cleanup(&pool, &[S_ROUNDTRIP]).await;
}

#[tokio::test]
async fn counterpart_layer_roundtrip() {
    let pool = connect_test_db().await;
    ensure_tables(&pool).await;
    cleanup(&pool, &[S_COUNTERPART]).await;

    let store = ChatMemoryStore::new(pool.clone());
    store
        .save_counterpart(
            S_COUNTERPART,
            C_ROUNDTRIP,
            json!({"客户联系人电话": "13800000000"}),
        )
        .await
        .expect("save counterpart");
    let loaded = store
        .load_counterpart(S_COUNTERPART, C_ROUNDTRIP)
        .await
        .expect("load counterpart");
    assert_eq!(loaded.get("客户联系人电话").unwrap(), "13800000000");

    cleanup(&pool, &[S_COUNTERPART]).await;
}

#[tokio::test]
async fn layers_and_scopes_are_isolated() {
    let pool = connect_test_db().await;
    ensure_tables(&pool).await;
    cleanup(&pool, &[S_ISOLATION_A, S_ISOLATION_B]).await;

    let store = ChatMemoryStore::new(pool.clone());
    store
        .save_subject(S_ISOLATION_A, json!({"主体": "A"}))
        .await
        .expect("save");
    store
        .save_subject(S_ISOLATION_B, json!({"主体": "B"}))
        .await
        .expect("save");
    store
        .save_counterpart(S_ISOLATION_A, C_ISOLATION_A1, json!({"对话方": "A1"}))
        .await
        .expect("save");
    store
        .save_counterpart(S_ISOLATION_A, C_ISOLATION_A2, json!({"对话方": "A2"}))
        .await
        .expect("save");

    // 跨主体隔离
    assert_eq!(
        store
            .load_subject(S_ISOLATION_A)
            .await
            .unwrap()
            .get("主体")
            .unwrap(),
        "A"
    );
    assert_eq!(
        store
            .load_subject(S_ISOLATION_B)
            .await
            .unwrap()
            .get("主体")
            .unwrap(),
        "B"
    );
    // 跨对话方隔离
    assert_eq!(
        store
            .load_counterpart(S_ISOLATION_A, C_ISOLATION_A1)
            .await
            .unwrap()
            .get("对话方")
            .unwrap(),
        "A1"
    );
    assert_eq!(
        store
            .load_counterpart(S_ISOLATION_A, C_ISOLATION_A2)
            .await
            .unwrap()
            .get("对话方")
            .unwrap(),
        "A2"
    );
    // 主体 B 无对话方记录 → 空对象（不与 A 串）
    assert_eq!(
        store
            .load_counterpart(S_ISOLATION_B, C_ISOLATION_A1)
            .await
            .unwrap(),
        json!({})
    );

    cleanup(&pool, &[S_ISOLATION_A, S_ISOLATION_B]).await;
}

#[tokio::test]
async fn version_increments_per_layer() {
    let pool = connect_test_db().await;
    ensure_tables(&pool).await;
    cleanup(&pool, &[S_VERSION]).await;

    let store = ChatMemoryStore::new(pool.clone());
    store
        .save_subject(S_VERSION, json!({"v": 1}))
        .await
        .expect("save");
    store
        .save_subject(S_VERSION, json!({"v": 2}))
        .await
        .expect("save");
    let version: i64 = sqlx::query_scalar(
        "SELECT version FROM isahl_auth.chat_ai_subject_memory WHERE subject_id = $1",
    )
    .bind(S_VERSION)
    .fetch_one(&pool)
    .await
    .expect("version");
    assert_eq!(version, 2, "主体层 version 应随写入递增");

    store
        .save_counterpart(S_VERSION, C_VERSION, json!({"v": 1}))
        .await
        .expect("save");
    store
        .save_counterpart(S_VERSION, C_VERSION, json!({"v": 2}))
        .await
        .expect("save");
    let version: i64 = sqlx::query_scalar(
        "SELECT version FROM isahl_auth.chat_ai_counterpart_memory
         WHERE subject_id = $1 AND counterpart_id = $2",
    )
    .bind(S_VERSION)
    .bind(C_VERSION)
    .fetch_one(&pool)
    .await
    .expect("version");
    assert_eq!(version, 2, "对话方层 version 应随写入递增");

    cleanup(&pool, &[S_VERSION]).await;
}
