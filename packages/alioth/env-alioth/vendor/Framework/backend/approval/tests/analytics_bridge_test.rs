//! fix-approval-engine-gap-closure D7 analytics 桥真值集成测试
//!
//! approver-workloads pending_count 只计非终态实例：专属审批人名下构造
//! 「已 approved（生命周期桥终态）+ 在途」实例 → pending_count 仅计在途；
//! 在途实例补 approved 桥后 → pending_count 归零（终态不再计入积压）。
//!
//! 审批人身份动态唯一（共享测试库防串扰：auth_users 行 + 实例同属本测试）。

use ::common::testing::connect_test_db;
use actix_web::{test, web, App};
use serde_json::Value;

mod common;
use common::setup_test_schema;

fn unique_approver() -> (i64, String) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let id = 460_000_000 + (nanos % 8_000_000) as i64;
    (id, format!("wl-{id}"))
}

async fn seed_approver(pool: &sqlx::PgPool, id: i64, username: &str) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, $2, $3, 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(username)
    .bind(format!("{username}@test.local"))
    .execute(pool)
    .await
    .unwrap();
}

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

async fn insert_instance(pool: &sqlx::PgPool, notice: &str, approver: i64) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, fk_subject, fk_operator, created_by_id)
           VALUES ($1, $2, $2, 1) RETURNING id"#,
    )
    .bind(notice)
    .bind(approver)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn mark_terminal(pool: &sqlx::PgPool, instance_id: i64, code: &str) {
    let sid = status_id(pool, code, "测试终态").await;
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

async fn pending_of_approver(body: &Value, name: &str) -> i64 {
    let rows = body["data"].as_array().expect("data 数组");
    let row = rows
        .iter()
        .find(|r| r["approverName"].as_str() == Some(name))
        .expect("应存在该审批人行");
    let pc = &row["pendingCount"];
    pc.as_i64()
        .or_else(|| pc.as_str().and_then(|s| s.parse::<i64>().ok()))
        .expect("pendingCount 数值")
}

#[tokio::test]
async fn workloads_pending_count_excludes_terminal_instances() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let (approver_id, approver_name) = unique_approver();
    seed_approver(&pool, approver_id, &approver_name).await;

    // 同一审批人名下：1 枚终态（approved）+ 1 枚在途
    let done = insert_instance(&pool, "已完成实例", approver_id).await;
    mark_terminal(&pool, done, "approved").await;
    let _open = insert_instance(&pool, "在途实例", approver_id).await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .configure(approval::handlers::analytics::register),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/analytics/approver-workloads")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "approver-workloads 应 200");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        pending_of_approver(&body, &approver_name).await,
        1,
        "pending_count 只计非终态（approved 终态实例不计入）: {body}"
    );

    // 新增在途实例计入；补终态桥后回落
    let open2 = insert_instance(&pool, "第二枚在途实例", approver_id).await;
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/analytics/approver-workloads")
            .to_request(),
    )
    .await;
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        pending_of_approver(&body, &approver_name).await,
        2,
        "新增在途实例应计入 pending: {body}"
    );
    mark_terminal(&pool, open2, "rejected").await;
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/analytics/approver-workloads")
            .to_request(),
    )
    .await;
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        pending_of_approver(&body, &approver_name).await,
        1,
        "终态化后 pending_count 应回落: {body}"
    );
}
