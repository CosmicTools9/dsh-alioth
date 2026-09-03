//! 驳回路由测试（2026-09-02 add-flow-reject-routing）：
//! - publish 物化 rejectAction/backTo；backTo 非法（下游/自身/越界）400
//! - runtime：驳回打回 → 在途重置 + 目标节点重开 → 重审迭代

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::{grant_user_access, setup_test_schema};

const USER_ID: i64 = 424901;

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx =
            ::common::context::RequestContext::with_username(USER_ID, "rr@test.local", "rr-test");
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

async fn seed_scope(pool: &PgPool) -> (i64, i64) {
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), '驳回路由-任务委派', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'RR-VIP', 1)
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
    .bind(format!("RR-{}", name))
    .bind(ctx_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn graph_with(back_to: Value) -> Value {
    json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approval", "label": "初审", "mode": "or_sign", "next": [{"to": 2}]},
            {"id": "b", "type": "approval", "label": "复审", "mode": "or_sign", "rejectAction": "back", "backTo": back_to, "next": [{"to": 3}]},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    })
}

async fn instance_count(pool: &PgPool, flow_id: i64, code: &str) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rro.code = $2 AND oe2.deleted_at IS NULL
             )"#,
    )
    .bind(flow_id)
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn first_pending(pool: &PgPool, flow_id: i64, code: &str) -> Option<i64> {
    sqlx::query_scalar(
        r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rro.code = $2 AND oe2.deleted_at IS NULL
             )
             AND NOT EXISTS (
               SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
               JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
               WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                 AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
             )
           ORDER BY oa.id LIMIT 1"#,
    )
    .bind(flow_id)
    .bind(code)
    .fetch_optional(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn reject_routing_publish_materializes_and_validates() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, _e) = seed_scope(&pool).await;

    // 合法：B back→A(1)
    let g = graph_with(json!(1));
    let flow = create_flow(&pool, "Ok", Some(scope_id), &g).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "合法 backTo 应发布: {b}");
    let cfg: Option<(Option<String>, Option<i64>)> = sqlx::query_as(
        r#"SELECT ea.timeline->>'rejectAction', NULLIF(ea.timeline->>'backTo','')::bigint
             FROM isahl."zc_id_even-approve" ea
             JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
             JOIN isahl."zc_id_process_rr_operation" rro
               ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
            WHERE rro.ref_left = $1 AND rro.code = 'b' AND ea.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        cfg,
        Some((Some("back".to_string()), Some(1))),
        "timeline 物化 rejectAction/backTo"
    );

    // 非法：backTo=自身（2）
    let g2 = graph_with(json!(2));
    let flow2 = create_flow(&pool, "Self", Some(scope_id), &g2).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow2}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "backTo=自身应拒: {b}");

    // 非法：backTo=下游 end(3)——end 不可达 B
    let g3 = graph_with(json!(3));
    let flow3 = create_flow(&pool, "Down", Some(scope_id), &g3).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow3}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "backTo=下游应拒: {b}");

    // 非法：越界
    let g4 = graph_with(json!(99));
    let flow4 = create_flow(&pool, "Oob", Some(scope_id), &g4).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow4}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "backTo 越界应拒: {b}");
}

#[tokio::test]
async fn reject_back_routes_to_target_and_rework_iterates() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_scope(&pool).await;

    let g = graph_with(json!(1));
    let flow = create_flow(&pool, "Rework", Some(scope_id), &g).await;
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow}/publish"),
        json!({})
    );
    assert_eq!(s, 200);
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");
    let a1 = first_pending(&pool, flow, "a").await.expect("A1 pending");
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-instances/{a1}/approve"),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200, "A1 approve 应 200");
    let b1 = first_pending(&pool, flow, "b").await.expect("B1 pending");
    assert_eq!(instance_count(&pool, flow, "a").await, 1);

    // B1 驳回 → 打回 A：新 A 实例（count=2），无在途
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{b1}/reject"),
        json!({"opinion": "材料不全"})
    );
    assert_eq!(s, 200, "B reject 应 200: {b}");
    assert_eq!(
        instance_count(&pool, flow, "a").await,
        2,
        "打回后 A 重开新实例"
    );
    let a2 = first_pending(&pool, flow, "a").await.expect("A2 pending");

    // 重审：A2 通过 → B2 生成 → B2 通过 → end 结论
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-instances/{a2}/approve"),
        json!({"opinion": "修改后同意"})
    );
    assert_eq!(s, 200);
    let b2 = first_pending(&pool, flow, "b").await.expect("B2 pending");
    assert_eq!(
        instance_count(&pool, flow, "b").await,
        2,
        "重审后 B 再次生成"
    );
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{b2}/approve"),
        json!({"opinion": "通过"})
    );
    assert_eq!(s, 200, "B2 approve 应 200: {b}");
    // end 结论物化
    let concl: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_statement" st
           WHERE st.deleted_at IS NULL AND st.tpl_id IS NOT NULL
             AND st.tpl_id IN (
               SELECT rs.ref_right FROM isahl."zc_id_operation_rr_statement" rs
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = rs.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rs.deleted_at IS NULL
             )"#,
    )
    .bind(flow)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(concl, 1, "重审通过后流程到 end 物化结论");
}
