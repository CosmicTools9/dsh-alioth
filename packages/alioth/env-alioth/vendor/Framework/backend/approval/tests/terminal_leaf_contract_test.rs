//! 终端叶表契约测试（2026-08-31 缺口补全，close-flow-definition-gaps）
//!
//! 锁定 2026-08-31 新契约行为：
//! - validate：event 驱动 start 缺 eventLeaf / 白名单外表名 → 拒绝（此前 event 臂为空，
//!   乱表名也 valid=true——不对称缺口）
//! - task 驱动 start 缺 taskLeaf → 拒绝（既有契约，回归守护）
//! - publish：event 驱动 start 物化事件真叶表范例行，类写入契约 `_f_='实现' _t_='范例'`
//!   （此前 EVENT 族无 INSERT 臂，行类 NULL——ALIOTH_ONTOLOGY_SPEC §4.3.3 违规形态）
//! - scope-options：terminal_leaves.event 含全部事件叶表（含 appr-* 审批事件子树）

mod common;

use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use common::setup_test_schema;
use serde_json::json;

use ::common::testing::connect_test_db;

/// validate：event 驱动 start 缺 eventLeaf → 拒绝（此前漏检）
#[tokio::test]
async fn validate_rejects_event_start_without_event_leaf() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "S"},
            {"id": "m", "type": "approve", "label": "A", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "E", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let app = test::init_service(
        App::new().service(web::scope("/test").configure(handlers::validate::register)),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/test/approval-flows/validate")
            .set_json(&graph)
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "event 驱动 start 缺 eventLeaf 必须拒绝（此前漏检）"
    );
}

/// validate：eventLeaf 白名单外表名 → 拒绝（此前漏检——乱表名 valid=true）
#[tokio::test]
async fn validate_rejects_event_leaf_outside_whitelist() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "S", "eventLeaf": "zc_id_not-a-real-table"},
            {"id": "m", "type": "approve", "label": "A", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "E", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let app = test::init_service(
        App::new().service(web::scope("/test").configure(handlers::validate::register)),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/test/approval-flows/validate")
            .set_json(&graph)
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "eventLeaf 白名单外表名必须拒绝（此前漏检）"
    );
}

/// validate：task 驱动 start 缺 taskLeaf → 拒绝（既有契约，回归守护）
#[tokio::test]
async fn validate_rejects_task_start_without_task_leaf() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "S", "drive": "task"},
            {"id": "m", "type": "approve", "label": "A", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "E", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let app = test::init_service(
        App::new().service(web::scope("/test").configure(handlers::validate::register)),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/test/approval-flows/validate")
            .set_json(&graph)
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "task 驱动 start 缺 taskLeaf 必须拒绝"
    );
}

/// publish：event 驱动 start 物化事件真叶表范例行，类契约 `_f_='实现' _t_='范例'`
#[tokio::test]
async fn publish_materializes_event_leaf_exemplar_with_class() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident"},
        ]
    });
    let user_id = 424248;
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'leaf-contract', 'leaf-contract', 'leaf-contract@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (notice, meta, code, created_by_id)
           VALUES ($1, $2::jsonb, 'draft', 1) RETURNING id"#,
    )
    .bind("终端叶表契约测试流程")
    .bind(graph.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    let ctx = ::common::context::RequestContext::with_username(
        user_id,
        "leaf-contract@test.local",
        "leaf-contract",
    );
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .wrap_fn(move |req, srv| {
                req.extensions_mut().insert(ctx.clone());
                srv.call(req)
            })
            .service(web::scope("/test").configure(handlers::publish::register)),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;
    let status = resp.status();
    let body = test::read_body(resp).await;
    assert!(
        status.is_success(),
        "发布应成功: {} {}",
        status,
        String::from_utf8_lossy(&body)
    );
    // 语义实体桥：rr_event → 事件真叶表范例行（2026-08-31 契约，非 even-approve 载体）
    let (f, t): (String, String) = sqlx::query_as(
        r#"SELECT n._f_, n._t_ FROM isahl."zc_id_even-accident" n
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = n.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           WHERE n.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(f, "实现", "范例行 _f_ 类契约（§4.3.3 形态 2）");
    assert_eq!(t, "范例", "范例行 _t_ 类契约（§4.3.3 形态 2）");
    // oper 范例行类契约：start 为自动节点 → oper-gate 范例行（实现·范例）
    let (of_, ot_): (String, String) = sqlx::query_as(
        r#"SELECT o._f_, o._t_ FROM isahl."zc_id_oper-gate" o
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = o.id AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           WHERE o.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(of_, "实现", "oper 范例行 _f_ 类契约（§4.3.3 形态 2）");
    assert_eq!(ot_, "范例", "oper 范例行 _t_ 类契约（§4.3.3 形态 2）");
}

/// scope-options：terminal_leaves.event 含全部事件叶表（含 appr-* 审批事件子树）
#[tokio::test]
async fn scope_options_event_leaves_include_approval_subtree() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(web::scope("/test").configure(handlers::scope_options::register)),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/scope-options")
            .to_request(),
    )
    .await;
    let status = resp.status();
    let body_bytes = test::read_body(resp).await;
    assert!(
        status.is_success(),
        "scope-options 应成功: {} {}",
        status,
        String::from_utf8_lossy(&body_bytes)
    );
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let data = &body["data"];
    let event = data["terminal_leaves"]["event"]
        .as_array()
        .expect("event leaves");
    let has_appr = event.iter().any(|i| {
        i["table"]
            .as_str()
            .map_or(false, |t| t.starts_with("zc_id_appr-"))
    });
    assert!(
        has_appr,
        "具体事件须含审批事件子树（2026-08-31 裁决：全族）"
    );
    let even = event
        .iter()
        .filter(|i| {
            i["table"]
                .as_str()
                .map_or(false, |t| t.starts_with("zc_id_even-"))
        })
        .count();
    assert_eq!(even, 8, "even 直叶 8 项");
}
