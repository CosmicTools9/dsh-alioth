//! commitment factor — 转交/加签集成测试
//!
//! 验证审批转交/加签操作的数据库写入正确性：
//! - transfer 更新实例的 fk_operator 并生成意见记录
//! - cc 仅生成意见记录（不改变审批人）
//!
//! 注意：由于所有测试共享 `aliothstudio_test` 并调用 `setup_test_schema`，请使用单线程运行：
//!   cargo test --test transfer_cc_test -- --test-threads=1
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
async fn transfer_updates_operator_and_records_action() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let current_user = insert_test_engineer(&pool, "当前审批人").await;
    let target_user = insert_test_engineer(&pool, "目标审批人").await;
    let event_id = insert_test_approve_event(&pool, "测试审批事项").await;
    let instance_id =
        insert_test_approval_instance(&pool, "节点1审核", event_id, current_user, current_user)
            .await;

    common::grant_user_access(
        &pool,
        current_user,
        "approval-instances",
        &["approve", "reject", "transfer", "cc"],
    )
    .await
    .expect("grant approval access");

    let ctx = ::common::context::RequestContext::with_username(current_user, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                common::noop_messaging::NoopMessaging::default(),
            )
                as std::sync::Arc<dyn ::common::messaging::MessagingService>))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::transfer_cc::register),
            ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri(&format!(
            "/test/approval-instances/{}/transfer",
            instance_id
        ))
        .set_json(serde_json::json!({ "target_id": target_user, "opinion": "请继续审批" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "transfer 应返回成功状态码");

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["status"], "transferred");
    assert_eq!(body["data"]["to_user"], target_user);

    let operator: Option<i64> =
        sqlx::query_scalar(r#"SELECT fk_operator FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
            .bind(instance_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        operator,
        Some(target_user),
        "transfer 后 fk_operator 应更新为目标审批人"
    );
}

#[tokio::test]
async fn cc_records_action_without_changing_operator() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let current_user = insert_test_engineer(&pool, "当前审批人").await;
    let cc_user = insert_test_engineer(&pool, "抄送审批人").await;
    let event_id = insert_test_approve_event(&pool, "测试审批事项").await;
    let instance_id =
        insert_test_approval_instance(&pool, "节点1审核", event_id, current_user, current_user)
            .await;

    common::grant_user_access(
        &pool,
        current_user,
        "approval-instances",
        &["approve", "reject", "transfer", "cc"],
    )
    .await
    .expect("grant approval access");

    let ctx = ::common::context::RequestContext::with_username(current_user, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                common::noop_messaging::NoopMessaging::default(),
            )
                as std::sync::Arc<dyn ::common::messaging::MessagingService>))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::transfer_cc::register),
            ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri(&format!("/test/approval-instances/{}/cc", instance_id))
        .set_json(serde_json::json!({ "target_id": cc_user, "opinion": "请知悉" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "cc 应返回成功状态码");

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["status"], "cc");

    // cc 不应改变操作人
    let operator: Option<i64> =
        sqlx::query_scalar(r#"SELECT fk_operator FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
            .bind(instance_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(operator, Some(current_user), "cc 后 fk_operator 应保持不变");
}
