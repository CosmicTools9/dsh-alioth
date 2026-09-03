//! Batch approve/reject 集成测试
//!
//! 验证批量审批端点的正确性：
//! - 批量通过多个实例
//! - 批量驳回多个实例
//! - 部分 ID 不存在时不会阻塞其余实例
//! - 返回准确的 processed/failed 计数
//!
//! 注意使用单线程运行：
//!   cargo test --test batch_approve_reject_test -- --test-threads=1

use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::Value;
use sqlx::PgPool;

mod common;
use common::setup_test_schema;

async fn insert_engineer(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_subj-employee" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_approve_event(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_instance(pool: &PgPool, node_name: &str, event_id: i64, fk_subject: i64) -> i64 {
    let instance_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, fk_subject, fk_operator, created_by_id)
           VALUES ($1, $2, $2, 1) RETURNING id"#,
    )
    .bind(node_name)
    .bind(fk_subject)
    .fetch_one(pool)
    .await
    .unwrap();
    // fk_approve 列已移除：实例↔审批事件经 rr_event 桥
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
async fn batch_approve_all_succeed() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let eng = insert_engineer(&pool, "batch测试人").await;
    let ev1 = insert_approve_event(&pool, "事件A").await;
    let ev2 = insert_approve_event(&pool, "事件B").await;
    let ev3 = insert_approve_event(&pool, "事件C").await;
    let id1 = insert_instance(&pool, "审批A", ev1, eng).await;
    let id2 = insert_instance(&pool, "审批B", ev2, eng).await;
    let id3 = insert_instance(&pool, "审批C", ev3, eng).await;

    common::grant_user_access(&pool, eng, "approval-instances", &["approve", "reject"])
        .await
        .expect("grant approval access");

    let ctx = ::common::context::RequestContext::with_username(eng, "actor@test", "actor");
    // fix-approval-action-chain P0-2：命名 bus + 订阅，断言批量走全链（桥+推进+事件）
    let bus: std::sync::Arc<dyn ::common::event_bus::DomainEventBus> =
        std::sync::Arc::new(::common::event_bus::InMemoryEventBus::new());
    let mut rx = bus.subscribe("ApprovalCompleted").await.expect("subscribe");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(bus.clone()))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::batch_approve_reject::register),
            ),
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/test/approval-instances/batch/approve")
        .set_json(serde_json::json!({
            "ids": [id1, id2, id3],
            "opinion": "批量通过"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["processed"], 3);
    assert_eq!(body["data"]["failed"].as_array().unwrap().len(), 0);

    // 全链断言 1：每个实例写入 approved 生命周期主状态桥
    for id in [id1, id2, id3] {
        let status: Option<String> = sqlx::query_scalar(
            r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
               JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
               WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL"#,
        )
        .bind(id)
        .fetch_optional(&pool)
        .await
        .unwrap()
        .flatten();
        assert_eq!(
            status.as_deref(),
            Some("approved"),
            "实例 {id} 缺 approved 桥"
        );
    }
    // 全链断言 2：每个实例各发布一次 ApprovalCompleted（result=approved）
    for _ in 0..3 {
        let ev = rx.try_recv().expect("每实例应各发一次 ApprovalCompleted");
        assert_eq!(ev.event_type, "ApprovalCompleted");
        assert_eq!(ev.payload["result"], "approved");
    }
}

#[tokio::test]
async fn batch_reject_all_succeed() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let eng = insert_engineer(&pool, "batch驳回测试人").await;
    let ev1 = insert_approve_event(&pool, "事件X").await;
    let ev2 = insert_approve_event(&pool, "事件Y").await;
    let id1 = insert_instance(&pool, "审批X", ev1, eng).await;
    let id2 = insert_instance(&pool, "审批Y", ev2, eng).await;

    common::grant_user_access(&pool, eng, "approval-instances", &["approve", "reject"])
        .await
        .expect("grant approval access");

    let ctx = ::common::context::RequestContext::with_username(eng, "actor@test", "actor");
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
                    .configure(handlers::batch_approve_reject::register),
            ),
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/test/approval-instances/batch/reject")
        .set_json(serde_json::json!({
            "ids": [id1, id2],
            "opinion": "批量驳回"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["processed"], 2);
    assert_eq!(body["data"]["failed"].as_array().unwrap().len(), 0);

    // fix-approval-action-chain P0-2/P0-3：批量驳回也须写 rejected 生命周期桥
    for id in [id1, id2] {
        let status: Option<String> = sqlx::query_scalar(
            r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
               JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
               WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL"#,
        )
        .bind(id)
        .fetch_optional(&pool)
        .await
        .unwrap()
        .flatten();
        assert_eq!(
            status.as_deref(),
            Some("rejected"),
            "实例 {id} 缺 rejected 桥"
        );
    }
}

#[tokio::test]
async fn batch_partial_failure_non_existent_id() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let eng = insert_engineer(&pool, "batch部分失败测试人").await;
    let ev = insert_approve_event(&pool, "事件P").await;
    let valid_id = insert_instance(&pool, "审批P", ev, eng).await;
    let nonexistent_id = 999999999;

    common::grant_user_access(&pool, eng, "approval-instances", &["approve", "reject"])
        .await
        .expect("grant approval access");

    let ctx = ::common::context::RequestContext::with_username(eng, "actor@test", "actor");
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
                    .configure(handlers::batch_approve_reject::register),
            ),
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/test/approval-instances/batch/approve")
        .set_json(serde_json::json!({
            "ids": [valid_id, nonexistent_id],
            "opinion": "部分通过"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["processed"], 1);
    assert_eq!(body["data"]["failed"].as_array().unwrap().len(), 1);
    let failed_id = body["data"]["failed"][0]["id"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| body["data"]["failed"][0]["id"].as_i64());
    assert_eq!(failed_id, Some(nonexistent_id));
}

#[tokio::test]
async fn batch_empty_ids_returns_zero_processed() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let eng = insert_engineer(&pool, "batch空测试人").await;

    common::grant_user_access(&pool, eng, "approval-instances", &["approve", "reject"])
        .await
        .expect("grant approval access");

    let ctx = ::common::context::RequestContext::with_username(eng, "actor@test", "actor");
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
                    .configure(handlers::batch_approve_reject::register),
            ),
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/test/approval-instances/batch/approve")
        .set_json(serde_json::json!({
            "ids": [],
            "opinion": ""
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["data"]["processed"], 0);
    assert_eq!(body["data"]["failed"].as_array().unwrap().len(), 0);
}
