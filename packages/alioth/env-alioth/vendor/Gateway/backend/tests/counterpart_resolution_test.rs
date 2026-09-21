//! 对话方解析集成测试（refactor-chat-ai-subject-identity-memory D-2/D5）。
//!
//! 覆盖 `memory_scope::CounterpartResolver::resolve` 的三条路径：
//! ① 参与方差集（新→旧回溯至含对话方的消息）→ 聚合到联系人；
//! ② 仅智能体侧参与方的消息被跳过（回溯）；
//! ③ 窗口内无对话方参与方 → 回退会话属主正向链（无绑定 ⇒ None，不臆造）。
//!
//! 数据自清：负 id 会话 + 唯一 code 联系方式。

use ::common::testing::connect_test_db;
use alioth_gateway::api::chat_sessions::adapters::db_message::SqlxMessageAdapter;
use alioth_gateway::api::chat_sessions::adapters::db_session::SqlxSessionAdapter;
use alioth_gateway::api::chat_sessions::memory_scope::CounterpartResolver;
use alioth_gateway::api::chat_sessions::ports::{MessageStorePort, SessionStorePort};
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};

const TEST_USER: i64 = -940001;
const CODES: [&str; 4] = [
    "cpres-agent-info",
    "cpres-cp-info",
    "cpres-cp-contact",
    "cpres-orphan-contact",
];
static REGISTRY_INIT: AtomicBool = AtomicBool::new(false);

async fn ensure_registry(pool: &PgPool) {
    if !REGISTRY_INIT.swap(true, Ordering::SeqCst) {
        let _ = trigger_registry::init::init_smart_registry_global(
            pool,
            trigger_registry::AppContainer::Gateway,
        )
        .await;
    }
}

async fn insert_info(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_info-isahl" (code, notice, dk_scene, dk_factor, dk_function)
           VALUES ($1, $1,
                   (SELECT id FROM isahl.zc_id_scene WHERE code = 'RR' AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor WHERE code = 'PFA' AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_MA' AND deleted_at IS NULL))
           RETURNING id"#,
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("insert contact info")
}

async fn insert_contact(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        // 类契约/坐标（§4.3.3 形态1 + §6.12）：经 ontology_binding 解析三坐标
        r#"INSERT INTO isahl.zc_id_contacts (code, notice, dk_scene, dk_factor, dk_function)
           VALUES ($1, $1,
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'JE'  AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'GEC' AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↑_DA' AND deleted_at IS NULL LIMIT 1)) RETURNING id"#,
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("insert contact")
}

async fn link(pool: &PgPool, contact_id: i64, info_id: i64) {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contacts_rr_infos" (ref_left, ref_right, default_info)
           VALUES ($1, $2, TRUE)"#,
    )
    .bind(contact_id)
    .bind(info_id)
    .execute(pool)
    .await
    .expect("link");
}

async fn cleanup(pool: &PgPool, sessions: &[i64]) {
    sqlx::query(r#"DELETE FROM isahl."zc_id_msgs-chat_ai" WHERE fk_thread = ANY($1)"#)
        .bind(sessions)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_thre-ai_session" WHERE id = ANY($1)"#)
        .bind(sessions)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_contacts_rr_infos" WHERE ref_right IN (
               SELECT id FROM isahl.zc_id_contact_infos WHERE code = ANY($1)
           )"#,
    )
    .bind(&CODES[..])
    .execute(pool)
    .await
    .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_contact_infos WHERE code = ANY($1)"#)
        .bind(&CODES[..])
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_contacts WHERE code = ANY($1)"#)
        .bind(&CODES[2..])
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn counterpart_resolution_paths() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    cleanup(&pool, &[]).await;

    let agent_info = insert_info(&pool, CODES[0]).await;
    let counterpart_info = insert_info(&pool, CODES[1]).await;
    let counterpart_contact = insert_contact(&pool, CODES[2]).await;
    link(&pool, counterpart_contact, counterpart_info).await;

    let session_store = SqlxSessionAdapter::new(pool.clone());
    let session = session_store
        .create_session("New Chat", None, TEST_USER)
        .await
        .expect("create session");

    let msg_store = SqlxMessageAdapter::new(pool.clone());
    // 较早：用户消息（发件人=对话方，收件人=智能体侧）
    let user_msg = msg_store
        .add_message(
            session.id,
            "帮我查一下",
            Some(counterpart_info),
            &[agent_info],
        )
        .await
        .expect("user msg");
    // 最新：AI 回复（仅智能体侧参与方 → 必须被回溯跳过）
    let ai_msg = msg_store
        .add_message(session.id, "好的", Some(agent_info), &[])
        .await
        .expect("ai msg");

    let resolver = CounterpartResolver::new(pool.clone());
    let resolved = resolver
        .resolve(session.id, TEST_USER, &[agent_info])
        .await
        .expect("resolve");
    assert_eq!(
        resolved,
        Some(counterpart_contact),
        "须回溯跳过仅智能体侧的消息，并从参与方联系方式聚合到对话方联系人"
    );

    // 空会话（无任何消息）→ 属主正向链回退；测试属主无实体绑定 ⇒ None（不臆造）
    let empty_session = session_store
        .create_session("New Chat", None, TEST_USER)
        .await
        .expect("create empty session");
    let resolved_empty = resolver
        .resolve(empty_session.id, TEST_USER, &[agent_info])
        .await
        .expect("resolve empty");
    assert_eq!(resolved_empty, None, "无参与方可解析且属主无绑定 ⇒ None");

    let all_sessions = vec![session.id, empty_session.id];
    cleanup(&pool, &all_sessions).await;
    let _ = (user_msg.id, ai_msg.id);
}
