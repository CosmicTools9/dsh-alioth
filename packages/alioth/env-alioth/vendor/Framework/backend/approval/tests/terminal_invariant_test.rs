//! fix-approval-engine-gap-closure D1 终态不变量集成测试
//!
//! ① 终态守卫：已 cancelled 实例再 approve → Validation 400 且不落新意见；
//! ② SLA 跳过终态实例：已 withdrawn 实例超时也不被 check_and_reject 改写
//!    （无驳回意见、桥保持 withdrawn）。

use ::common::testing::connect_test_db;
use ::common::SYSTEM_USER_ID;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use serde_json::Value;
use std::sync::Arc;

mod common;
use common::{grant_user_access, setup_test_schema};

const USER_ID: i64 = 442701;

macro_rules! build_app {
    ($pool:expr) => {{
        let ctx =
            ::common::context::RequestContext::with_username(USER_ID, "tguard@test", "tguard");
        actix_web::test::init_service(
            App::new()
                .app_data(web::Data::new($pool.clone()))
                .app_data(web::Data::new(
                    Arc::new(::common::event_bus::InMemoryEventBus::new())
                        as Arc<dyn ::common::event_bus::DomainEventBus>,
                ))
                .service(
                    web::scope("/test")
                        .wrap_fn(move |req, srv| {
                            req.extensions_mut().insert(ctx.clone());
                            srv.call(req)
                        })
                        .configure(approval::handlers::approve_reject::register),
                ),
        )
        .await
    }};
}

async fn test_user(pool: &sqlx::PgPool) -> i64 {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'tguard', 'tguard', 'tguard@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
    // 员工行（审批实例 fk_subject/fk_operator 落员工域）
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_subj-employee" (notice, created_by_id)
           VALUES ('tguard-员工', 1) RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_approve_event(pool: &sqlx::PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_instance(pool: &sqlx::PgPool, notice: &str, event_id: i64, subject: i64) -> i64 {
    let instance_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, fk_subject, fk_operator, created_by_id)
           VALUES ($1, $2, $2, 1) RETURNING id"#,
    )
    .bind(notice)
    .bind(subject)
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

/// stus-approve 字典行 find-or-create（update_lifecycle_status 同构）
async fn status_id(pool: &sqlx::PgPool, code: &str, notice: &str) -> i64 {
    match sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT id FROM isahl."zc_id_stus-approve" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .unwrap()
    .flatten()
    {
        Some(id) => id,
        None => sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl."zc_id_stus-approve" (id, code, notice)
               VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"#,
        )
        .bind(code)
        .bind(notice)
        .fetch_one(pool)
        .await
        .unwrap(),
    }
}

/// 直插生命周期桥（audit_outbox 三态行创建同构）
async fn set_bridge(pool: &sqlx::PgPool, instance_id: i64, code: &str, notice: &str) {
    let sid = status_id(pool, code, notice).await;
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
           VALUES (isahl.gen_next_zuid(), $1, $2)"#,
    )
    .bind(instance_id)
    .bind(sid)
    .execute(pool)
    .await
    .unwrap();
}

async fn bridge_code(pool: &sqlx::PgPool, instance_id: i64) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
           JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
           WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .unwrap()
    .flatten()
}

async fn reject_opinion_count(pool: &sqlx::PgPool, instance_id: i64) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM isahl."zc_id_deta-opinion"
           WHERE fk_list = $1 AND notice = '审批驳回' AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// D1.1：终态（cancelled）实例再 approve → Validation，不落意见、桥不变
#[tokio::test]
async fn approve_cancelled_instance_blocked_by_terminal_guard() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let eng_id = test_user(&pool).await;
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve", "reject"])
        .await
        .expect("grant approval access");
    let event_id = insert_approve_event(&pool, "终态守卫测试事件").await;
    let instance_id = insert_instance(&pool, "终态守卫测试实例", event_id, eng_id).await;
    set_bridge(&pool, instance_id, "cancelled", "已取消").await;

    let app = build_app!(pool);
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-instances/{}/approve", instance_id))
            .set_json(serde_json::json!({ "opinion": "补批" }))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "终态实例再 approve 应 400 Validation"
    );
    let body: Value = test::read_body_json(resp).await;
    let msg =
        body["error"].as_str().unwrap_or("").to_string() + body["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("已终态") || msg.contains("cancelled"),
        "错误信息应含终态守卫：{msg}"
    );
    // 不落任何新意见（approve 写意见前置守卫）
    let opinions: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_deta-opinion"
           WHERE fk_list = $1 AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(opinions, 0, "守卫拦截后不应产生意见");
    assert_eq!(
        bridge_code(&pool, instance_id).await.as_deref(),
        Some("cancelled"),
        "桥应保持 cancelled"
    );
}

/// D1.3：SLA 轮询跳过终态（withdrawn）实例——不写驳回意见、桥不变
#[tokio::test]
async fn sla_check_skips_withdrawn_instance() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    // NGAC：check_and_reject 内系统身份 require_resource_access（跳过分支不写，授予无害）
    grant_user_access(&pool, SYSTEM_USER_ID, "approval_actions", &["create"])
        .await
        .expect("grant system approval access");

    let sla_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-duration" (notice, mark, created_by_id)
           VALUES ('SLA 守卫时长', $1, 1) RETURNING id"#,
    )
    .bind(rust_decimal::Decimal::from(1))
    .fetch_one(&pool)
    .await
    .unwrap();
    let event_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, qk_sla, created_by_id)
           VALUES ('SLA 守卫事件', $1, 1) RETURNING id"#,
    )
    .bind(sla_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (notice, created_at, created_by_id)
           VALUES ('SLA 守卫实例', NOW() - interval '2 hours', 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(event_id)
    .execute(&pool)
    .await
    .unwrap();
    set_bridge(&pool, instance_id, "withdrawn", "已撤回").await;

    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());
    approval::sla_timeout::check_and_reject(&pool, &bus)
        .await
        .expect("check_and_reject 应成功");

    assert_eq!(
        reject_opinion_count(&pool, instance_id).await,
        0,
        "withdrawn 终态实例不应被 SLA 驳回"
    );
    assert_eq!(
        bridge_code(&pool, instance_id).await.as_deref(),
        Some("withdrawn"),
        "桥应保持 withdrawn"
    );
}
