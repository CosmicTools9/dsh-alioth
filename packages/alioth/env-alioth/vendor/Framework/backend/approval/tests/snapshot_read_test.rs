//! fix-approval-engine-gap-closure D4 发布快照读取集成测试
//!
//! 发布后篡改 process.meta 设计草稿（condition expr 翻转、start 节点移位），
//! 运行时（initiate 起点解析 / 条件选边）仍按发布版本快照（timeline.graph）
//! 求值——快照缺失才回退 process.meta。

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use std::sync::Arc;

mod common;
use common::{ensure_role_member, setup_test_schema};

const USER_ID: i64 = 443901;

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
           VALUES ($1, 'sn-test', 'sn-test', 'sn@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
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
           VALUES (isahl.gen_next_zuid(), 'SN-委派定义', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '快照实体', 'VIP', 1) RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (scope_id, entity_id)
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

/// 篡改设计草稿：condition expr 翻转为常假、start 图内 id 改写成不存在的节点
async fn sabotage_draft_meta(pool: &sqlx::PgPool, flow_id: i64) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_proc-approve"
           SET meta = jsonb_set(
                 jsonb_set(meta, '{nodes,1,expr}', '"1 == 0"', false),
                 '{nodes,0,id}', '"n-sabotaged"', false)
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .execute(pool)
    .await
    .unwrap();
}

/// 运行时锚定发布快照：草稿被篡改（expr 常假 / start id 不存在）后发起仍走
/// 快照 start 起点 + 快照 condition（code=='VIP' → VIP 审批实例）。
#[tokio::test]
async fn runtime_uses_published_snapshot_not_draft_meta() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-cond", "type": "condition", "label": "VIP 判断",
             "expr": "code == 'VIP'",
             "next": [{"to": 2, "cond": "code == 'VIP'"}, {"to": 3}]},
            {"id": "n-vip", "type": "approve", "label": "VIP 审批"},
            {"id": "n-other", "type": "approve", "label": "普通审批"},
            {"id": "n-end", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "快照读取流程", scope_id, &graph).await;

    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let ctx = ::common::context::RequestContext::with_username(USER_ID, "sn@test.local", "sn-test");
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

    // 篡改草稿 meta（发布版本快照不受影响）
    sabotage_draft_meta(&pool, flow_id).await;

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(
        s, 200,
        "快照锚定下 initiate 应 200（草稿被篡改也不影响）: {b}"
    );
    let ids = b["data"]["instance_ids"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(ids.len(), 1, "应仅创建 VIP 分支一枚审批实例: {b}");
    let inst: i64 = ids[0].as_str().unwrap().parse().unwrap();

    // 实例落位 = 快照 condition 的 VIP 分支（节点载体 notice = 'VIP 审批'）
    let node_label: String = sqlx::query_scalar(
        r#"SELECT ea.notice FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           JOIN isahl."zc_id_even-approve" ea ON ea.id = oe.ref_right AND ea.deleted_at IS NULL
           WHERE oa.id = $1
             AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                         JOIN isahl."zc_id_process_rr_operation" rro2
                           ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                         WHERE oe2.ref_right = ea.id AND oe2.deleted_at IS NULL)
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(inst)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        node_label, "VIP 审批",
        "按快照 expr 应路由至 VIP 审批（草稿 expr 已翻转常假）"
    );
}
