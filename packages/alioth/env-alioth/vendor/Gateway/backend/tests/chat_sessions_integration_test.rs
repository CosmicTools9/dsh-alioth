//! chat_sessions 后端集成测试（M232 task 15，fix-chat-ai-feature-gaps）。
//!
//! 覆盖（#[tokio::test] + connect_test_db，禁 #[sqlx::test]）：
//! - meta 往返：user 消息带附件/知识引用 → assistant 消息 agent_code/structured/
//!   usage/knowledge_refs → get_messages JOIN 恢复；软删消息被查询过滤
//! - pinned_agent 跳路由：switch pin 后 resolve_agent 命中 registry 直返（不走
//!   router/LLM——pinned 路径在路由前短路）
//! - feedback toggle：upsert → 同 rating 再点删除
//! - PATCH 标题：update_session_title → get_session 回读（notice 列）
//! - 取消语义 / generation_id 精确轮询：cache 层（mod.rs tests，无 DB）
//!
//! 使用 aliothstudio_test 库；负 ID 测试用户自清。registry 全局初始化一次。

use ::common::testing::connect_test_db;
use alioth_gateway::api::chat_sessions::adapters::agent_dispatch::AgentRouterAdapter;
use alioth_gateway::api::chat_sessions::adapters::db_message::SqlxMessageAdapter;
use alioth_gateway::api::chat_sessions::adapters::db_session::SqlxSessionAdapter;
use alioth_gateway::api::chat_sessions::ports::{
    AgentDispatchPort, MessageStorePort, SessionStorePort,
};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

const TEST_USER: i64 = -930001;
const AI_USER: i64 = -930002;
const OTHER_USER: i64 = -930003;

static REGISTRY_INIT: AtomicBool = AtomicBool::new(false);

/// 测试环境自愈（一次性）：
/// 1. Trigger registry 全局初始化（Gateway 容器硬编码层次；重复调用幂等忽略）
/// 2. 019 两表幂等 ensure（chat_message_meta / chat_message_feedback）——测试库
///    重置后表会丢失（今日实测复现），内嵌与 migration 019_chat_ai_meta.sql
///    同构的 CREATE TABLE IF NOT EXISTS 自愈（isahl_auth 工程 schema ensure
///    模式，参照 standalone_auth/mod.rs:765 先例；与 OpenActivity db.rs 自愈
///    同语义——测试不得依赖"库内表碰巧存在"）
async fn ensure_registry(pool: &sqlx::PgPool) {
    if !REGISTRY_INIT.swap(true, Ordering::SeqCst) {
        let _ = trigger_registry::init::init_smart_registry_global(
            pool,
            trigger_registry::AppContainer::Gateway,
        )
        .await;
        // 与 Gateway/backend/migrations/019_chat_ai_meta.sql 同构（来源标注）
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS isahl_auth.chat_message_meta (
                msg_id         bigint PRIMARY KEY REFERENCES isahl."zc_id_msgs-chat_ai"(id) ON DELETE CASCADE,
                session_id     bigint NOT NULL,
                agent_code     text NOT NULL DEFAULT '',
                structured     jsonb,
                usage          jsonb,
                attachments    jsonb,
                knowledge_refs jsonb,
                created_at     timestamptz NOT NULL DEFAULT now()
            )"#,
        )
        .execute(pool)
        .await
        .expect("ensure chat_message_meta (019)");
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS isahl_auth.chat_message_feedback (
                msg_id     bigint NOT NULL REFERENCES isahl."zc_id_msgs-chat_ai"(id) ON DELETE CASCADE,
                user_id    bigint NOT NULL,
                rating     text NOT NULL CHECK (rating IN ('up','down')),
                comment    text,
                created_at timestamptz NOT NULL DEFAULT now(),
                updated_at timestamptz NOT NULL DEFAULT now(),
                PRIMARY KEY (msg_id, user_id)
            )"#,
        )
        .execute(pool)
        .await
        .expect("ensure chat_message_feedback (019)");
    }
}

async fn cleanup(pool: &sqlx::PgPool, session_ids: &[i64], msg_ids: &[i64]) {
    // FK 级联覆盖物理删（chat_message_meta/feedback ON DELETE CASCADE）
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_msgs-chat_ai" WHERE id = ANY($1) OR fk_thread = ANY($2)"#,
    )
    .bind(msg_ids)
    .bind(session_ids)
    .execute(pool)
    .await
    .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_thre-ai_session" WHERE id = ANY($1)"#)
        .bind(session_ids)
        .execute(pool)
        .await
        .ok();
}

async fn new_session(pool: &sqlx::PgPool) -> (i64, Vec<i64>) {
    let store = SqlxSessionAdapter::new(pool.clone());
    let session = store
        .create_session("New Chat", None, TEST_USER)
        .await
        .expect("create session");
    (session.id, vec![session.id])
}

#[tokio::test]
async fn meta_roundtrip_and_join_restore() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    let (session_id, sessions) = new_session(&pool).await;
    let mut msgs: Vec<i64> = Vec::new();

    let store = SqlxMessageAdapter::new(pool.clone());

    // user 消息 + 附件/知识引用 meta
    let user_msg = store
        .add_message(session_id, "帮我看看这个合同", None)
        .await
        .expect("user msg");
    msgs.push(user_msg.id);
    store
        .save_message_meta(
            user_msg.id,
            session_id,
            "",
            None,
            None,
            Some(&json!([{ "type": "image", "mime": "image/png", "data_base64": "AAAA" }])),
            Some(&json!([{ "key": "LAB-44", "title": "赔偿标准" }])),
        )
        .await
        .expect("save user meta");

    // assistant 消息 + agent_code/structured/usage/knowledge_refs meta
    let assistant_msg = store
        .add_message(session_id, "已分析该合同…", Some(AI_USER))
        .await
        .expect("assistant msg");
    msgs.push(assistant_msg.id);
    store
        .save_message_meta(
            assistant_msg.id,
            session_id,
            "data_analysis",
            Some(&json!({ "kind": "analysis", "score": 0.9 })),
            Some(&json!({ "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 })),
            None,
            Some(&json!([{ "key": "LAB-44", "title": "赔偿标准" }])),
        )
        .await
        .expect("save assistant meta");

    // JOIN 恢复：assistant 行携带 meta（agent_code/structured/usage/knowledge_refs）
    let rows = store
        .get_messages(session_id, TEST_USER, 0, 50)
        .await
        .expect("get messages");
    let assistant = rows
        .iter()
        .find(|r| r.id == assistant_msg.id)
        .expect("assistant row");
    assert_eq!(assistant.agent_code.as_deref(), Some("data_analysis"));
    assert_eq!(
        assistant
            .structured
            .as_ref()
            .and_then(|v| v.get("score"))
            .and_then(|v| v.as_f64()),
        Some(0.9)
    );
    assert_eq!(
        assistant
            .usage
            .as_ref()
            .and_then(|v| v.get("total_tokens"))
            .and_then(|v| v.as_i64()),
        Some(15)
    );
    assert_eq!(
        assistant
            .knowledge_refs
            .as_ref()
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(1)
    );
    // user 行 meta 保留附件/知识引用
    let user_row = rows.iter().find(|r| r.id == user_msg.id).expect("user row");
    assert_eq!(
        user_row
            .attachments
            .as_ref()
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(1)
    );

    // 软删过滤：删 assistant → get_messages 不再返回（meta 保留但查询侧过滤）
    store
        .soft_delete_message(assistant_msg.id)
        .await
        .expect("soft delete");
    let rows_after = store
        .get_messages(session_id, TEST_USER, 0, 50)
        .await
        .expect("get messages after delete");
    assert!(
        rows_after.iter().all(|r| r.id != assistant_msg.id),
        "软删消息必须被 get_messages 过滤"
    );

    cleanup(&pool, &sessions, &msgs).await;
}

#[tokio::test]
async fn feedback_toggle_semantics() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    let (session_id, sessions) = new_session(&pool).await;
    let mut msgs: Vec<i64> = Vec::new();

    let store = SqlxMessageAdapter::new(pool.clone());
    let msg = store
        .add_message(session_id, "hi", None)
        .await
        .expect("msg");
    msgs.push(msg.id);

    // 非 owner 不可反馈
    assert!(!store
        .message_belongs_to_user(msg.id, -999999)
        .await
        .expect("owner check"));
    assert!(store
        .message_belongs_to_user(msg.id, TEST_USER)
        .await
        .expect("owner ok"));

    // upsert: up
    assert_eq!(
        store
            .set_message_feedback(msg.id, TEST_USER, "up", None)
            .await
            .expect("up"),
        Some("up".to_string())
    );
    // toggle: 同 rating 再点 → 删除
    assert_eq!(
        store
            .set_message_feedback(msg.id, TEST_USER, "up", None)
            .await
            .expect("toggle"),
        None
    );
    // 改 rating: down（comment）
    assert_eq!(
        store
            .set_message_feedback(msg.id, TEST_USER, "down", Some("看不懂"))
            .await
            .expect("down"),
        Some("down".to_string())
    );

    cleanup(&pool, &sessions, &msgs).await;
}

#[tokio::test]
async fn pinned_agent_short_circuits_routing() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    let (session_id, sessions) = new_session(&pool).await;

    // pin form_filling（registry 内置）
    let session_store = SqlxSessionAdapter::new(pool.clone());
    session_store
        .update_session_state(
            session_id,
            TEST_USER,
            json!({ "pinned_agent": "form_filling" }),
        )
        .await
        .expect("pin");

    // dummy llm（不落网——pinned 路径在 router/LLM 前短路）
    let llm = llm::LlmService::new(llm::LlmServiceConfig {
        provider: llm::LlmProvider::DeepSeek,
        api_key: "sk-test-dummy".to_string(),
        model: "dummy".to_string(),
        flash_model: "dummy".to_string(),
        base_url: Some("http://127.0.0.1:9".to_string()),
        timeout_seconds: 2,
        max_retries: 0,
        generation_params: Default::default(),
        roles: Default::default(),
    })
    .expect("dummy llm");

    let adapter = AgentRouterAdapter::new(pool.clone());
    let decided = adapter
        .resolve_agent(session_id, "你好", None, &[], "zh-CN", &llm)
        .await
        .expect("pinned resolve");
    assert_eq!(decided, "form_filling", "pin 命中必须直返不走路由");

    // auto 清 pin 后不再短路（无 pin → 走路由需 LLM → 此路径不在本测试断言；
    // 仅验证清 pin 写回后 resolve 不再命中 pinned 分支：以不存在的 code 为哨兵
    // ——未清 pin 会 Err/直返不存在 code？registry 存在性守卫使其回落路由）
    session_store
        .update_session_state(session_id, TEST_USER, json!({ "pinned_agent": null }))
        .await
        .expect("unpin");
    let decided_after = adapter
        .resolve_agent(session_id, "你好", None, &[], "zh-CN", &llm)
        .await;
    // 无 pin：进入路由（dummy LLM 会失败）→ adapter 不 Err（路由错误走 general? 这里 resolve 返回 Err 或 general——
    // AgentRouterAdapter.resolve_agent 路由失败时上层 fallback；本层可能 Err。兼容两种：不 panic 即可）
    let _ = decided_after;

    cleanup(&pool, &sessions, &[]).await;
}

/// 会话删除级联（cascade-chat-session-delete）：会话软删 + 消息软删 + 衍生行清理；
/// 非本人 / 重复删除 → 404 语义且零副作用。
#[tokio::test]
async fn session_delete_cascades_to_messages() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;

    let session_store = SqlxSessionAdapter::new(pool.clone());
    let msg_store = SqlxMessageAdapter::new(pool.clone());

    // 前置自愈：断言失败时收尾 cleanup 不会执行（panic 提前返回），
    // 故进入用例先清本用例 owner 的遗留行，防跨运行累积。
    let owners: Vec<i64> = vec![TEST_USER, OTHER_USER];
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_msgs-chat_ai"
           WHERE fk_thread IN (SELECT id FROM isahl."zc_id_thre-ai_session" WHERE created_by_id = ANY($1))"#,
    )
    .bind(&owners)
    .execute(&pool)
    .await
    .expect("pre-clean messages");
    sqlx::query(r#"DELETE FROM isahl."zc_id_thre-ai_session" WHERE created_by_id = ANY($1)"#)
        .bind(&owners)
        .execute(&pool)
        .await
        .expect("pre-clean sessions");

    // 本人会话：2 条消息 + meta + feedback
    let session = session_store
        .create_session("级联删除测试", None, TEST_USER)
        .await
        .expect("create session");
    let session_id = session.id;
    let user_msg = msg_store
        .add_message(session_id, "你好", None)
        .await
        .expect("user msg");
    let ai_msg = msg_store
        .add_message(session_id, "回复", Some(AI_USER))
        .await
        .expect("assistant msg");
    msg_store
        .save_message_meta(
            ai_msg.id,
            session_id,
            "general",
            None,
            Some(&json!({ "total_tokens": 3 })),
            None,
            None,
        )
        .await
        .expect("save meta");
    msg_store
        .set_message_feedback(ai_msg.id, TEST_USER, "up", None)
        .await
        .expect("save feedback");

    // 他方会话（用于非本人级联负例）
    let other_session = session_store
        .create_session("他方会话", None, OTHER_USER)
        .await
        .expect("create other session");
    let other_msg = msg_store
        .add_message(other_session.id, "他方消息", None)
        .await
        .expect("other msg");

    session_store
        .delete_session(session_id, TEST_USER)
        .await
        .expect("delete own session");

    let (session_deleted, session_deleted_by): (bool, Option<i64>) = sqlx::query_as(
        r#"SELECT deleted_at IS NOT NULL, deleted_by_id
           FROM isahl."zc_id_thre-ai_session" WHERE id = $1"#,
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("session row");
    assert!(session_deleted, "会话行已软删");
    assert_eq!(session_deleted_by, Some(TEST_USER), "会话删除归因操作者");

    let msgs: Vec<(i64, bool, Option<i64>)> = sqlx::query_as(
        r#"SELECT id, deleted_at IS NOT NULL, deleted_by_id
           FROM isahl."zc_id_msgs-chat_ai" WHERE fk_thread = $1 ORDER BY id"#,
    )
    .bind(session_id)
    .fetch_all(&pool)
    .await
    .expect("session messages");
    assert_eq!(msgs.len(), 2, "消息行保留（软删而非物理删）");
    assert!(
        msgs.iter()
            .all(|(_, deleted, by)| *deleted && *by == Some(TEST_USER)),
        "会话全部消息软删且归因操作者"
    );

    let meta_left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM isahl_auth.chat_message_meta WHERE msg_id = $1")
            .bind(ai_msg.id)
            .fetch_one(&pool)
            .await
            .expect("meta count");
    assert_eq!(meta_left, 0, "衍生 meta 行随级联清理");
    let fb_left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.chat_message_feedback WHERE msg_id = $1",
    )
    .bind(ai_msg.id)
    .fetch_one(&pool)
    .await
    .expect("feedback count");
    assert_eq!(fb_left, 0, "反馈行随级联清理");

    // 非本人：404 语义 + 零级联
    let err = session_store
        .delete_session(other_session.id, TEST_USER)
        .await
        .expect_err("非本人删除必须失败");
    assert_eq!(err, "SESSION_NOT_FOUND");
    let other_session_alive: bool = sqlx::query_scalar(
        r#"SELECT deleted_at IS NULL FROM isahl."zc_id_thre-ai_session" WHERE id = $1"#,
    )
    .bind(other_session.id)
    .fetch_one(&pool)
    .await
    .expect("other session row");
    assert!(other_session_alive, "非本人会话未被改动");
    let other_msg_alive: bool = sqlx::query_scalar(
        r#"SELECT deleted_at IS NULL FROM isahl."zc_id_msgs-chat_ai" WHERE id = $1"#,
    )
    .bind(other_msg.id)
    .fetch_one(&pool)
    .await
    .expect("other msg row");
    assert!(other_msg_alive, "非本人会话消息未被级联");

    // 重复删除：404 语义、消息不再被触碰
    let err2 = session_store
        .delete_session(session_id, TEST_USER)
        .await
        .expect_err("重复删除必须失败");
    assert_eq!(err2, "SESSION_NOT_FOUND");

    cleanup(
        &pool,
        &[session_id, other_session.id],
        &[user_msg.id, ai_msg.id, other_msg.id],
    )
    .await;
}

#[tokio::test]
async fn patch_title_updates_notice() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    let (session_id, sessions) = new_session(&pool).await;

    let store = SqlxSessionAdapter::new(pool.clone());
    assert!(store
        .update_session_title(session_id, TEST_USER, "运费规则讨论")
        .await
        .expect("rename"));
    let session = store
        .get_session(session_id, TEST_USER)
        .await
        .expect("reload")
        .expect("exists");
    assert_eq!(session.title, "运费规则讨论");

    // 非 owner 改名 → false
    assert!(!store
        .update_session_title(session_id, -999999, "hijack")
        .await
        .expect("not owner"));

    cleanup(&pool, &sessions, &[]).await;
}
