//! subflow wait=true 同步续链测试（2026-09-02 A4）：
//! - publish：wait 物化 + wait 要求 target 含 end（无 end 400）
//! - runtime：父流程停于 subflow → 子流程终局（end 物化）回调续推父流程

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::{grant_user_access, setup_test_schema};

const USER_ID: i64 = 425101;

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx =
            ::common::context::RequestContext::with_username(USER_ID, "sw@test.local", "sw-test");
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
           VALUES (isahl.gen_next_zuid(), '子流程等待-任务委派', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'SW-VIP', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (scope_id, entity_id)
}

async fn create_flow(
    pool: &PgPool,
    name: &str,
    code: &str,
    ctx_id: Option<i64>,
    graph: &Value,
) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, fk_context, created_by_id, _f_, _t_)
           VALUES ($1, $2::jsonb, $3, $4, 1, '实现', '范例') RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(code)
    .bind(ctx_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn pending_first(pool: &PgPool, flow_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND oe2.deleted_at IS NULL
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
    .fetch_optional(pool)
    .await
    .unwrap()
    .expect("pending instance")
}

async fn instance_total(pool: &PgPool, flow_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND oe2.deleted_at IS NULL
             )"#,
    )
    .bind(flow_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn subflow_wait_publish_requires_end_in_target() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, _e) = seed_scope(&pool).await;

    // 子流程 A 含 end（合法 target）；子流程 B 无 end
    let child_ok = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approval", "label": "子审批", "mode": "or_sign", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let child_no_end = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approval", "label": "子审批", "mode": "or_sign"}
        ]
    });
    let code_ok = format!(
        "SW-CHILD-OK-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let cf = create_flow(&pool, "子流程-OK", &code_ok, Some(scope_id), &child_ok).await;
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{cf}/publish"),
        json!({})
    );
    assert_eq!(s, 200);
    let cf2 = create_flow(
        &pool,
        "子流程-NoEnd",
        "SW-CHILD-NOEND",
        Some(scope_id),
        &child_no_end,
    )
    .await;
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{cf2}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    // wait=true + 含 end target → 200 且物化 wait
    let parent = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "sub", "type": "subflow", "label": "子流程", "wait": true, "target": code_ok, "next": [{"to": 2}]},
            {"id": "b", "type": "approval", "label": "父后续", "mode": "or_sign"}
        ]
    });
    let pf = create_flow(&pool, "父流程", "SW-PARENT", Some(scope_id), &parent).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "wait+含 end target 应发布: {b}");
    let wait: Option<String> = sqlx::query_scalar(
        r#"SELECT ea.timeline->>'wait' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl."zc_id_process_rr_operation" rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'sub' AND ea.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(pf)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(wait.as_deref(), Some("true"), "wait 物化");

    // wait=true + 无 end target → 400
    let parent2 = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "sub", "type": "subflow", "label": "子流程", "wait": true, "target": "SW-CHILD-NOEND"}
        ]
    });
    let pf2 = create_flow(
        &pool,
        "父流程-Bad",
        "SW-PARENT-BAD",
        Some(scope_id),
        &parent2,
    )
    .await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf2}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "wait 要求 target 含 end: {b}");
}

#[tokio::test]
async fn subflow_wait_resumes_parent_after_child_end() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_scope(&pool).await;

    let child = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approval", "label": "子审批", "mode": "or_sign", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let code_c = format!(
        "SW-C-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let cf = create_flow(&pool, "子流程", &code_c, Some(scope_id), &child).await;
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{cf}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    let parent = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "sub", "type": "subflow", "label": "子流程", "wait": true, "target": code_c, "next": [{"to": 2}]},
            {"id": "b", "type": "approval", "label": "父后续", "mode": "or_sign"}
        ]
    });
    let pf = create_flow(&pool, "父流程", "SW-P", Some(scope_id), &parent).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "父流程 publish: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "父 initiate: {b}");
    // 父流程停于 subflow：父后续未建实例；子流程首链已建（waiting）
    assert_eq!(instance_total(&pool, pf).await, 0, "父后续应等待");
    assert_eq!(instance_total(&pool, cf).await, 1, "子流程首链已创建");

    // 子流程审批通过 → end → 回调续推父流程 → 父后续实例出现
    let ca = pending_first(&pool, cf).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{ca}/approve"),
        json!({"opinion": "子流程同意"})
    );
    assert_eq!(s, 200, "子流程 approve: {b}");
    assert_eq!(
        instance_total(&pool, pf).await,
        1,
        "子流程终局后父流程续推创建父后续实例"
    );
    let pb = pending_first(&pool, pf).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{pb}/approve"),
        json!({"opinion": "父通过"})
    );
    assert_eq!(s, 200, "父后续 approve: {b}");
}
