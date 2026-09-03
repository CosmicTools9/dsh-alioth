//! 网关语义测试（2026-09-02 fix-flow-gateway-semantics）：
//! - condition routing=exclusive：首个 cond=true 边首中即止；inclusive 全命中（存量兼容）
//! - branch joinRule=all 局部汇聚：只等本节点入边源分支，多并行区互不串扰
//! - publish 物化 routing；非法 routing 拒绝

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::{grant_user_access, setup_test_schema};

const USER_ID: i64 = 424801;

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx = ::common::context::RequestContext::with_username(
            USER_ID,
            "gw-sem@test.local",
            "gw-sem-test",
        );
        test::init_service(
            App::new()
                .app_data(web::Data::new($pool.clone()))
                .app_data(web::Data::new($bus))
                .service(
                    web::scope("/test")
                        .wrap_fn(move |req, srv| {
                            req.extensions_mut().insert(ctx.clone());
                            srv.call(req)
                        })
                        .configure(handlers::publish::register)
                        .configure(handlers::initiate::register)
                        .configure(handlers::approve_reject::register),
                ),
        )
        .await
    }};
}

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
        let status: u16 = resp.status().as_u16();
        let body: Value = test::read_body_json(resp).await;
        (status, body)
    }};
}

async fn seed_task_commission(pool: &PgPool) -> (i64, i64) {
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), '网关-任务委派', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'GWSEM-VIP', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (scope_id, entity_id)
}

async fn create_flow(pool: &PgPool, name: &str, ctx_id: Option<i64>, graph: &Value) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, fk_context, created_by_id, _f_, _t_)
           VALUES ($1, $2::jsonb, $3, $4, 1, '实现', '范例') RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(format!("GWSEM-{}", name))
    .bind(ctx_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 某节点 rro.code 下已建审批实例数（人工目标扇出验证；静态 SQL）
async fn instances_of(pool: &PgPool, flow_id: i64, code: &str) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.tpl_id IS NOT NULL AND oa.deleted_at IS NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rro.code = $2
                 AND oe2.deleted_at IS NULL
             )"#,
    )
    .bind(flow_id)
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn approve_one(pool: &PgPool, flow_id: i64, code: &str) -> i64 {
    // 取该节点一个 pending 实例并 approve（VOTER 无岗位 → NULL operator，resource 授权即可）
    let inst: i64 = sqlx::query_scalar(
        r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.tpl_id IS NOT NULL AND oa.deleted_at IS NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rro.code = $2 AND oe2.deleted_at IS NULL
             )
           ORDER BY oa.id LIMIT 1"#,
    )
    .bind(flow_id)
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap();
    inst
}

#[tokio::test]
async fn condition_exclusive_first_match_wins() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // 两条 cond 均 true：exclusive 只走首条（x）
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "c", "type": "condition", "label": "路由", "routing": "exclusive",
             "next": [{"to": 2, "cond": "1 == 1"}, {"to": 3, "cond": "1 == 1"}]},
            {"id": "x", "type": "approval", "label": "首中分支", "mode": "or_sign"},
            {"id": "y", "type": "approval", "label": "次分支", "mode": "or_sign"}
        ]
    });
    let flow_id = create_flow(&pool, "Excl", Some(scope_id), &graph).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");

    // routing 物化到载体 timeline
    let routing: Option<String> = sqlx::query_scalar(
        r#"SELECT ea.timeline->>'routing' FROM isahl."zc_id_even-approve" ea
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl."zc_id_process_rr_operation" rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'c' AND ea.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(
        routing.as_deref(),
        Some("exclusive"),
        "timeline 应物化 routing=exclusive"
    );

    assert_eq!(
        instances_of(&pool, flow_id, "x").await,
        1,
        "首中分支 x 建实例"
    );
    assert_eq!(
        instances_of(&pool, flow_id, "y").await,
        0,
        "exclusive 不得扇出 y"
    );
}

#[tokio::test]
async fn condition_inclusive_all_matches_run_legacy() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // 无 routing（存量）→ inclusive：两条 true 全扇出
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "c", "type": "condition", "label": "路由",
             "next": [{"to": 2, "cond": "1 == 1"}, {"to": 3, "cond": "1 == 1"}]},
            {"id": "x", "type": "approval", "label": "分支 X", "mode": "or_sign"},
            {"id": "y", "type": "approval", "label": "分支 Y", "mode": "or_sign"}
        ]
    });
    let flow_id = create_flow(&pool, "Incl", Some(scope_id), &graph).await;
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200");
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200");
    assert_eq!(
        instances_of(&pool, flow_id, "x").await,
        1,
        "inclusive x 建实例"
    );
    assert_eq!(
        instances_of(&pool, flow_id, "y").await,
        1,
        "inclusive y 建实例"
    );
}

#[tokio::test]
async fn condition_invalid_routing_rejected() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, _entity_id) = seed_task_commission(&pool).await;

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "c", "type": "condition", "label": "路由", "routing": "random",
             "next": [{"to": 2, "cond": "1 == 1"}]},
            {"id": "x", "type": "approval", "label": "分支", "mode": "or_sign"}
        ]
    });
    let flow_id = create_flow(&pool, "BadRouting", Some(scope_id), &graph).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "非法 routing 发布应被拒: {b}");
}

#[tokio::test]
async fn branch_local_join_does_not_wait_other_region() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // 双并行区：start→{p1→a1→j1→c1, p2→b1→j2}；j1 只等 a1（不入边 b1）
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}, {"to": 4}]},
            {"id": "p1", "type": "parallel", "label": "区A并行", "next": [{"to": 2}]},
            {"id": "a1", "type": "approval", "label": "区A审批", "mode": "or_sign", "next": [{"to": 3}]},
            {"id": "j1", "type": "branch", "label": "区A汇聚", "joinRule": "all", "next": [{"to": 7}]},
            {"id": "p2", "type": "parallel", "label": "区B并行", "next": [{"to": 5}]},
            {"id": "b1", "type": "approval", "label": "区B审批", "mode": "or_sign", "next": [{"to": 6}]},
            {"id": "j2", "type": "branch", "label": "区B汇聚", "joinRule": "all"},
            {"id": "c1", "type": "approval", "label": "区A后续", "mode": "or_sign"}
        ]
    });
    let flow_id = create_flow(&pool, "LocalJoin", Some(scope_id), &graph).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200");
    // a1/b1 均建实例（两区并行）
    assert_eq!(instances_of(&pool, flow_id, "a1").await, 1);
    assert_eq!(instances_of(&pool, flow_id, "b1").await, 1);
    assert_eq!(instances_of(&pool, flow_id, "c1").await, 0);

    // a1 通过 → j1(all) 只等 a1（入边源）→ 放行建 c1；b1 仍在途不受影响
    let a1 = approve_one(&pool, flow_id, "a1").await;
    let bus2: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app2 = build_app!(pool, bus2);
    let (s, b) = post_json!(
        &app2,
        &format!("/test/approval-instances/{a1}/approve"),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200, "approve a1 应 200: {b}");
    assert_eq!(
        instances_of(&pool, flow_id, "c1").await,
        1,
        "局部汇聚：b1 在途不得阻塞区 A 汇聚（flow 级等待会在此卡住）"
    );
    assert_eq!(instances_of(&pool, flow_id, "b1").await, 1, "B 区在途保持");
}
