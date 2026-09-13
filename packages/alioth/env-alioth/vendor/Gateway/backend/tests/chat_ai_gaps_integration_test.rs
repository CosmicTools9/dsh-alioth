//! chat-ai capability-gaps M-D 集成测试（fix-chat-ai-capability-gaps）。
//!
//! 覆盖（#[tokio::test] + connect_test_db，禁 #[sqlx::test]）：
//! - C4 draft_context 读写：update_session_state 重写式落库 → get_session_context
//!   回读（content 整体替换非 append）；与 turn_count 等并行键 jsonb|| 合并不丢失
//!   （draft/memory 同轮双跑并发契约）
//! - C8 动作审计落库：GatewayActionHandler（工具路径真实执行点）执行动作后
//!   isahl_audit.audit_events 含 operation=chat_ai.action.execute 记录——
//!   成功 Permit（metadata action_type/target_count/confirmed:false）、
//!   失败 Deny（metadata.error）；user_email 自 auth_users 解析
//!
//! 使用 aliothstudio_test 库；审计 user_id 须为正（record_audit_event 校验），
//! 自建正 ID 用户 + 负 ID 会话属主，用例自清。

use ::common::testing::connect_test_db;
use ai_agent::agents::tool_orchestrator::ActionHandler;
use ai_agent::tools::ToolContext;
use alioth_gateway::api::chat_sessions::adapters::db_session::SqlxSessionAdapter;
use alioth_gateway::api::chat_sessions::adapters::tool_bridge::GatewayActionHandler;
use alioth_gateway::api::chat_sessions::ports::SessionStorePort;
use alioth_gateway::i18n::init_i18n_manager;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static REGISTRY_INIT: AtomicBool = AtomicBool::new(false);

/// 测试环境自愈（一次性）：Trigger registry 全局初始化（Gateway 容器硬编码
/// 层次；重复调用幂等忽略——create_session 触发会话表 trigger 依赖此注册）。
/// 与 chat_sessions_integration_test.rs 同构；本文件用例只触会话表与审计表，
/// 无需 019 消息 meta 表 ensure。
async fn ensure_registry(pool: &sqlx::PgPool) {
    if !REGISTRY_INIT.swap(true, Ordering::SeqCst) {
        let _ = trigger_registry::init::init_smart_registry_global(
            pool,
            trigger_registry::AppContainer::Gateway,
        )
        .await;
    }
}

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// 建正 ID 审计用户（auth_users.email 供 resolve_user_email 解析）
async fn ensure_audit_user(pool: &sqlx::PgPool, uid: i64) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at)
           VALUES ($1, $2, $3, $4, 'active', true, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE SET email = EXCLUDED.email, is_active = true"#,
    )
    .bind(uid)
    .bind(format!("Audit User {}", uid))
    .bind(format!("audit_{}", uid))
    .bind(format!("audit_{}@test.local", uid))
    .execute(pool)
    .await
    .expect("insert audit user");
}

/// 清理：审计行 → 站内信 → 会话 → 用户（FK 顺序）
async fn cleanup(pool: &sqlx::PgPool, audit_uid: i64, owner: i64, session_ids: &[i64]) {
    sqlx::query(
        r#"DELETE FROM isahl_audit.audit_events
           WHERE user_id = $1 AND operation = 'chat_ai.action.execute'"#,
    )
    .bind(audit_uid)
    .execute(pool)
    .await
    .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_msgs-system" WHERE created_by_id = $1"#)
        .bind(audit_uid)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_thre-ai_session" WHERE id = ANY($1)"#)
        .bind(session_ids)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(audit_uid)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(owner)
        .execute(pool)
        .await
        .ok();
}

async fn new_session(pool: &sqlx::PgPool, owner: i64) -> i64 {
    let store = SqlxSessionAdapter::new(pool.clone());
    let session = store
        .create_session("New Chat", None, owner)
        .await
        .expect("create session");
    session.id
}

/// GatewayActionHandler 直接执行（工具路径真实执行点；i18n 仅 generate_document
/// 的 AI contact 命名用，本测试动作不触达——空管理器即可）
async fn handler_execute(
    pool: &sqlx::PgPool,
    session_id: i64,
    user_id: i64,
    action_type: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let handler: Arc<dyn ActionHandler> =
        Arc::new(GatewayActionHandler::new(pool.clone(), init_i18n_manager()));
    let ctx = ToolContext {
        session_id,
        user_id: Some(user_id),
        db_pool: pool.clone(),
        allowed_schemas: vec![],
        action_handler: None,
    };
    handler.execute(action_type, &[], &params, &ctx).await
}

/// 最近一条 chat_ai.action.execute 审计行
async fn last_action_audit(
    pool: &sqlx::PgPool,
    user_id: i64,
) -> (String, String, String, serde_json::Value) {
    sqlx::query_as::<_, (String, String, String, serde_json::Value)>(
        r#"SELECT user_email, decision, object_path, metadata
           FROM isahl_audit.audit_events
           WHERE user_id = $1 AND operation = 'chat_ai.action.execute'
           ORDER BY created_at DESC, id DESC LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("audit row exists")
}

// ── C4: draft_context 读写 ───────────────────────────────────────────────────

#[tokio::test]
async fn draft_context_rewrite_persist_and_merge() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    let owner: i64 = -950101;
    let sid = new_session(&pool, owner).await;
    let store = SqlxSessionAdapter::new(pool.clone());
    let state_of =
        |agent_state: Option<serde_json::Value>| agent_state.expect("agent_state exists");

    // 首轮落库：draft_context + 任务状态（与 R5 各后台任务同 patch 语义）
    store
        .update_session_state(
            sid,
            owner,
            json!({
                "draft_context": {
                    "content": "- 进行中：运费分摊规则确认",
                    "updated_at": 1000,
                    "source_turn_count": 4
                },
                "draft_context_state": { "status": "ok", "last_attempt_at": 1000 }
            }),
        )
        .await
        .expect("write draft v1");
    let (_, agent_state) = store
        .get_session_context(sid, owner)
        .await
        .expect("read back v1");
    let st = state_of(agent_state);
    assert_eq!(st["draft_context"]["content"], "- 进行中：运费分摊规则确认");
    assert_eq!(st["draft_context"]["source_turn_count"], 4);
    assert_eq!(st["draft_context_state"]["status"], "ok");

    // 并行键写入（模拟 memory 通道同轮落库）——jsonb|| 按 key 合并互不覆盖
    store
        .update_session_state(sid, owner, json!({ "turn_count": 8 }))
        .await
        .expect("write turn_count");
    // 重写式更新：整体替换 draft_context（非 append）
    store
        .update_session_state(
            sid,
            owner,
            json!({
                "draft_context": {
                    "content": "- 已确认：运费分摊 6:4\n- 新事项：核对 8 月对账单",
                    "updated_at": 2000,
                    "source_turn_count": 8
                },
                "draft_context_state": { "status": "ok", "last_attempt_at": 2000 }
            }),
        )
        .await
        .expect("rewrite draft v2");

    let (_, agent_state) = store
        .get_session_context(sid, owner)
        .await
        .expect("read back v2");
    let st = state_of(agent_state);
    let content = st["draft_context"]["content"].as_str().expect("content");
    // 重写式：v2 全文覆盖 v1，无拼接残留
    assert_eq!(content, "- 已确认：运费分摊 6:4\n- 新事项：核对 8 月对账单");
    assert!(!content.contains("规则确认"), "旧内容不得残留（非 append）");
    assert_eq!(st["draft_context"]["source_turn_count"], 8);
    assert_eq!(st["draft_context"]["updated_at"], 2000);
    // 并行键合并不丢（draft/memory 双通道并发契约）
    assert_eq!(st["turn_count"], 8);
    assert_eq!(st["draft_context_state"]["status"], "ok");

    cleanup(&pool, 0, owner, &[sid]).await;
}

// ── C8: 动作审计落库 ─────────────────────────────────────────────────────────

#[tokio::test]
async fn action_execution_audits_permit_with_metadata() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    let audit_uid: i64 = 9_500_001;
    let owner: i64 = -950201;
    ensure_audit_user(&pool, audit_uid).await;
    let sid = new_session(&pool, owner).await;

    // send_notification（Preview 级，无确认门禁）→ 真实执行成功
    handler_execute(
        &pool,
        sid,
        audit_uid,
        "send_notification",
        json!({ "title": "测试通知", "content": "审计落库验证" }),
    )
    .await
    .expect("send_notification 应执行成功");

    let (email, decision, object_path, metadata) = last_action_audit(&pool, audit_uid).await;
    assert_eq!(decision, "permit", "成功执行 → Permit");
    assert_eq!(
        object_path,
        format!(
            "chat-sessions/{}/actions/agent_action:send_notification",
            sid
        ),
        "工具路径 object_path"
    );
    assert_eq!(
        email,
        format!("audit_{}@test.local", audit_uid),
        "auth_users email 解析"
    );
    assert_eq!(metadata["action_type"], "send_notification");
    assert_eq!(metadata["target_count"], 0);
    assert_eq!(metadata["confirmed"], false, "工具路径无用户确认通道");

    cleanup(&pool, audit_uid, owner, &[sid]).await;
}

#[tokio::test]
async fn action_execution_audits_deny_on_failure() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    let audit_uid: i64 = 9_500_002;
    let owner: i64 = -950202;
    ensure_audit_user(&pool, audit_uid).await;
    let sid = new_session(&pool, owner).await;

    // 参数缺失 → handler 失败 → Deny + error
    let outcome = handler_execute(&pool, sid, audit_uid, "send_notification", json!({})).await;
    assert!(outcome.is_err(), "缺 title/content 必须失败");

    let (email, decision, object_path, metadata) = last_action_audit(&pool, audit_uid).await;
    assert_eq!(decision, "deny", "执行失败 → Deny");
    assert_eq!(
        object_path,
        format!(
            "chat-sessions/{}/actions/agent_action:send_notification",
            sid
        )
    );
    assert_eq!(email, format!("audit_{}@test.local", audit_uid));
    assert_eq!(metadata["action_type"], "send_notification");
    let err = metadata["error"].as_str().expect("error 必须入 metadata");
    assert!(err.contains("MISSING_PARAM"), "error 内容: {}", err);

    cleanup(&pool, audit_uid, owner, &[sid]).await;
}
