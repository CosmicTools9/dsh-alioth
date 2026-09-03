//! cc 结构化收件人测试（2026-09-02 A6）：
//! - publish：recipientRefs 物化为结构化数组；非法 kind 400
//! - runtime：推进至 cc 发布 ApprovalCc（payload recipients 数组 + resolvedUsers）

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::{ensure_role_member, setup_test_schema};

const USER_ID: i64 = 425201;
const CC_USER: i64 = 425202;

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx =
            ::common::context::RequestContext::with_username(USER_ID, "ccs@test.local", "ccs-test");
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
                        .configure(handlers::initiate::register),
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
    // 收件人用户（employee 实体：subj-employee.notice → fk_user；无 engineer 实体）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-employee"
           (id, code, notice, fk_user, created_by_id, created_at, updated_at)
           VALUES (isahl.gen_next_zuid(), 'cc-emp', 'cc-target', $1, 1, NOW(), NOW())"#,
    )
    .bind(CC_USER)
    .execute(pool)
    .await
    .unwrap();
    // 收件人用户（engineer 解析按 username/name）
    ensure_role_member(pool, "default", CC_USER).await.unwrap();
    sqlx::query(
        r#"UPDATE isahl_auth.auth_users SET username = 'cc-target', name = 'cc-target'
           WHERE id = $1"#,
    )
    .bind(CC_USER)
    .execute(pool)
    .await
    .unwrap();
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'cc结构化-任务委派', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'CCS-VIP', 1)
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
    .bind(format!("CCS-{}", name))
    .bind(ctx_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn cc_graph(refs: Value) -> Value {
    json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "c", "type": "cc", "label": "抄送", "recipientRefs": refs},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    })
}

#[tokio::test]
async fn cc_structured_publish_materializes_array_and_validates() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, _e) = seed_scope(&pool).await;

    let g = cc_graph(json!([{"kind": "employee", "id": "cc-target"}]));
    let flow = create_flow(&pool, "Structured", Some(scope_id), &g).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "结构化收件人应发布: {b}");
    let recips: Option<Value> = sqlx::query_scalar(
        r#"SELECT ea.timeline->'recipients' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl."zc_id_process_rr_operation" rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'c' AND ea.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    let recips = recips.expect("timeline recipients");
    assert!(recips.is_array(), "物化应为数组: {recips}");
    assert_eq!(recips[0]["kind"], "employee");
    assert_eq!(recips[0]["id"], "cc-target");

    // 非法 kind → 400
    let g2 = cc_graph(json!([{"kind": "owner", "id": "x"}]));
    let flow2 = create_flow(&pool, "BadRef", Some(scope_id), &g2).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow2}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "非法 recipientRefs 应拒: {b}");
}

#[tokio::test]
async fn cc_structured_event_resolves_recipients() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let mut rx = bus.subscribe("ApprovalCc").await.unwrap();
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_scope(&pool).await;

    let g = cc_graph(json!([
        {"kind": "employee", "id": "cc-target"},
        {"kind": "role", "id": "default"}
    ]));
    let flow = create_flow(&pool, "Event", Some(scope_id), &g).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish: {b}");
    eprintln!("CCDBG pre-initiate");
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate: {b}");
    eprintln!("CCDBG post-initiate");

    let evt = match tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await {
        Ok(Ok(evt)) => evt,
        other => {
            eprintln!("CCDBG recv timeout/other: {:?}", other);
            panic!("ApprovalCc 事件未达");
        }
    };
    assert_eq!(evt.event_type, "ApprovalCc");
    let payload = &evt.payload;
    assert!(
        payload["recipients"].is_array(),
        "payload recipients 结构化: {payload}"
    );
    assert_eq!(payload["recipients"].as_array().map(|a| a.len()), Some(2));
    // engineer 按 username/name 解析 + role default 成员（CC_USER 已在 default）
    let resolved = payload["resolvedUsers"].as_array().expect("resolvedUsers");
    let ids: Vec<i64> = resolved.iter().filter_map(|v| v.as_i64()).collect();
    assert!(ids.contains(&CC_USER), "cc-target 用户应被解析: {ids:?}");
}
