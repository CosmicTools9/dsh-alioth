//! fix-approval-engine-gap-closure D2 loop cursor 执行域隔离集成测试
//!
//! 发布侧：node.maxIter=2 被采纳为 vars.maxIter；cursors 初始空对象
//! （legacy flat cursor 弃写）。
//! 运行时：实体发起首跳链驱动 loop 单步迭代（start→loop 自动节点链），
//! cursor 以执行域键（实体行 id.to_string()）写入——断言键精确等于实体 id、
//! 值与 maxIter 语义一致（0→1），且无 flat cursor 残留。
//!
//! 注：同一流程模板的后续执行会被汇聚等待语义（branch-join flow 级在途回退，
//! 引擎既有行为，非 D2 范围）挡在循环体外——本测试验证 D2 契约本身
//! （域键读写 + maxIter 采纳 + 初始空），不涉跨执行并发。

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use std::sync::Arc;

mod common;
use common::{ensure_role_member, setup_test_schema};

const USER_ID: i64 = 443801;

macro_rules! post_json {
    ($app:expr, $uri:expr, $body:expr) => {{
        let resp = test::call_service(
            $app,
            test::TestRequest::post()
                .uri($uri)
                .set_json($body)
                .to_request(),
        )
        .await;
        let _s: u16 = resp.status().as_u16();
        let b: Value = test::read_body_json(resp).await;
        (_s, b)
    }};
}

async fn test_user(pool: &sqlx::PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'lp-test', 'lp-test', 'lp@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
    ensure_role_member(pool, "default", USER_ID).await.unwrap();
}

async fn seed_task_commission(pool: &sqlx::PgPool) -> (i64, i64) {
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'LP-委派定义', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let e1: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '委派实体一', 'C1', 1) RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (scope_id, e1)
}

async fn create_flow(pool: &sqlx::PgPool, name: &str, ctx_id: i64, graph: &Value) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, fk_context, created_by_id, _f_, _t_)
           VALUES ($1, $2::jsonb, 'draft', $3, 1, '实现', '范例') RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(ctx_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 读取 loop 节点 operation.meta.loop（cursors/vars）
async fn loop_meta(pool: &sqlx::PgPool, flow_id: i64) -> serde_json::Value {
    sqlx::query_scalar::<_, Option<serde_json::Value>>(
        r#"SELECT o.meta->'loop' FROM isahl."zc_id_operation" o
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = o.id
           WHERE rro.ref_left = $1 AND rro.code = 'n-loop' AND o.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_optional(pool)
    .await
    .unwrap()
    .flatten()
    .unwrap_or(serde_json::Value::Null)
}

#[tokio::test]
async fn loop_cursor_entity_keyed_with_node_maxiter() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, e1) = seed_task_commission(&pool).await;

    // maxIter=2（node.maxIter 采纳路径）；loopVars 不声明 maxIter
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-loop", "type": "loop", "label": "循环", "loopFormula": "cursor < maxIter",
             "loopVars": [], "maxIter": 2,
             "next": [{"to": 2}, {"to": 3}]},
            {"id": "n-approve", "type": "approve", "label": "循环体审批", "next": [{"to": 1}]},
            {"id": "n-end", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "循环游标域键", scope_id, &graph).await;

    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let ctx = ::common::context::RequestContext::with_username(USER_ID, "lp@test.local", "lp-test");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(bus))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::publish::register)
                    .configure(handlers::initiate::register),
            ),
    )
    .await;

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    // D2-1：发布即采纳 node.maxIter=2；cursors 初始空对象（flat cursor 弃写）
    let lm = loop_meta(&pool, flow_id).await;
    assert_eq!(lm["vars"]["maxIter"], 2, "node.maxIter=2 应被采纳");
    assert_eq!(lm["cursors"], json!({}), "cursors 应初始空对象");
    assert!(lm.get("cursor").is_none(), "flat cursor 不再写");

    // D2-2：实体发起 → 首跳链驱动 loop 单步：cursor 以实体 id 字符串为键 =1
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": e1})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");
    let lm = loop_meta(&pool, flow_id).await;
    assert_eq!(
        lm["cursors"][e1.to_string()],
        1,
        "cursor 应以实体行 id 为执行域键写入: {lm}"
    );
    assert_eq!(
        lm["cursors"].as_object().map(|m| m.len()),
        Some(1),
        "不应出现其他执行域键: {lm}"
    );
    assert!(lm.get("cursor").is_none(), "写入后仍无 flat cursor: {lm}");
}
