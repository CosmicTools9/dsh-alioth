//! commitment factor — approve/reject 集成测试
//!
//! 验证审批通过/驳回操作的数据库写入正确性：
//! - 调用 handler 创建 zc_id_deta-opinion 意见记录
//! - 返回 JSON 状态标记
//!
//! 注意：由于所有测试共享 `aliothstudio_test` 并调用 `setup_test_schema`，请使用单线程运行：
//!   cargo test --test approve_reject_test -- --test-threads=1
//!
//! 数据依赖：
//! - zc_id_subj-employee（工程师）
//! - zc_id_even-approve（审批事件）
//! - zc_id_oper-approve（审批实例）
//! - zc_id_deta-opinion（审批意见）

use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::Value;
use sqlx::PgPool;

mod common;
use common::setup_test_schema;

async fn insert_test_engineer(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_subj-employee" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_test_approve_event(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_test_approval_instance(
    pool: &PgPool,
    node_name: &str,
    event_id: i64,
    fk_subject: i64,
    fk_operator: i64,
) -> i64 {
    let instance_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, fk_subject, fk_operator, created_by_id)
           VALUES ($1, $2, $3, 1) RETURNING id"#,
    )
    .bind(node_name)
    .bind(fk_subject)
    .bind(fk_operator)
    .fetch_one(pool)
    .await
    .unwrap();
    // fk_approve 列已移除：实例↔审批事件经 operation_rr_event 桥
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(event_id)
    .execute(pool)
    .await
    .unwrap();
    instance_id
}

#[tokio::test]
async fn approve_creates_action_and_returns_status() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let eng_id = insert_test_engineer(&pool, "审批测试工程师").await;
    let approve_event_id = insert_test_approve_event(&pool, "测试审批事项").await;
    let instance_id =
        insert_test_approval_instance(&pool, "节点1审核", approve_event_id, eng_id, eng_id).await;

    // NGAC：授予测试用户 approval-instances 的 approve/reject 权限
    common::grant_user_access(&pool, eng_id, "approval-instances", &["approve", "reject"])
        .await
        .expect("grant approval access");

    let ctx = ::common::context::RequestContext::with_username(eng_id, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                ::common::event_bus::InMemoryEventBus::new(),
            )
                as std::sync::Arc<dyn ::common::event_bus::DomainEventBus>))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::approve_reject::register),
            ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri(&format!("/test/approval-instances/{}/approve", instance_id))
        .set_json(serde_json::json!({ "opinion": "同意" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "approve 应返回成功状态码");

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["status"], "approved");

    // 审批意见的 fk_list 指向实例 ID（而非 fk_approve 事件），见 approve_reject.rs 注释
    let (notice, comments): (String, String) = sqlx::query_as(
        r#"SELECT notice, opinion FROM isahl."zc_id_deta-opinion"
           WHERE fk_list = $1 AND deleted_at IS NULL
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(notice, "审批通过");
    assert_eq!(comments, "同意");
}

#[tokio::test]
async fn reject_creates_action_and_returns_status() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let eng_id = insert_test_engineer(&pool, "审批测试工程师").await;
    let approve_event_id = insert_test_approve_event(&pool, "测试审批事项").await;
    let instance_id =
        insert_test_approval_instance(&pool, "节点1审核", approve_event_id, eng_id, eng_id).await;

    // NGAC：授予测试用户 approval-instances 的 approve/reject 权限
    common::grant_user_access(&pool, eng_id, "approval-instances", &["approve", "reject"])
        .await
        .expect("grant approval access");

    let ctx = ::common::context::RequestContext::with_username(eng_id, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                ::common::event_bus::InMemoryEventBus::new(),
            )
                as std::sync::Arc<dyn ::common::event_bus::DomainEventBus>))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::approve_reject::register),
            ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri(&format!("/test/approval-instances/{}/reject", instance_id))
        .set_json(serde_json::json!({ "opinion": "不同意" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "reject 应返回成功状态码");

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["status"], "rejected");

    // fix-approval-action-chain P0-3：手动驳回必须写 rejected 生命周期主状态桥
    // （此前 reject 只写意见+事件，桥缺失导致状态双源漂移）。
    let bridge: Option<String> = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
           JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
           WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(bridge.as_deref(), Some("rejected"), "reject 缺 rejected 桥");
}
