//! 多账号共享标识的解析语义集成测试（allow-duplicate-email-accounts）
//!
//! 覆盖链路：同一邮箱注册两个账号（MUST 201）→ 以共享邮箱登录返回择账号挑战
//! （200 + candidates，MUST NOT 签发令牌/建会话）→ 密码全不匹配 401 且 MUST NOT
//! 增加任何候选的失败计数 → 按所选 username 走单列路径登录成功。

mod common;

use actix_web::{test, web, App};
use serde_json::json;
use sqlx::PgPool;

async fn setup_pool() -> PgPool {
    ::common::testing::connect_test_db().await
}

#[tokio::test]
async fn shared_email_login_returns_account_selection_challenge() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::login::configure)),
    )
    .await;

    let email = format!("shared-{}@test.local", uuid::Uuid::new_v4().simple());
    let pass_a = "SharedPass1!";
    let pass_b = "SharedPass2!";
    let user_a = format!("shared_a_{}", uuid::Uuid::new_v4().simple());
    let user_b = format!("shared_b_{}", uuid::Uuid::new_v4().simple());

    // 1. 同一邮箱注册两个账号：MUST 均 201（email 非唯一身份基点）
    for (username, password) in [(&user_a, pass_a), (&user_b, pass_b)] {
        let req = test::TestRequest::post()
            .uri("/auth/register")
            .set_json(json!({ "email": email, "username": username, "password": password }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status().as_u16(),
            201,
            "共享邮箱注册 MUST 成功（{username}）"
        );
    }

    // 注册落 pending_approval → 激活后方可登录
    sqlx::query("UPDATE isahl_auth.auth_users SET status = 'active' WHERE email = $1")
        .bind(&email)
        .execute(&pool)
        .await
        .unwrap();

    // 2. 以共享邮箱登录：200 择账号挑战（无令牌、无会话、候选含两账号）
    let req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({ "identifier": email, "password": pass_a }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "多账号 MUST 返回择账号挑战");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["username_selection_required"], true);
    assert!(body["access_token"].is_null(), "挑战 MUST NOT 签发令牌");
    assert!(body["session_id"].is_null(), "挑战 MUST NOT 建立会话");
    let candidates: Vec<String> = body["candidates"]
        .as_array()
        .expect("candidates MUST 为数组")
        .iter()
        .filter_map(|c| c["username"].as_str().map(str::to_string))
        .collect();
    assert!(
        candidates.contains(&user_a) && candidates.contains(&user_b),
        "候选 MUST 含两个 username，实际 {candidates:?}"
    );

    // 3. 密码不匹配任何候选：401 且 MUST NOT 增加任何候选的失败计数
    let req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({ "identifier": email, "password": "WrongPass123!" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401, "全不匹配 MUST 401");
    let counters: Vec<i32> = sqlx::query_scalar(
        "SELECT COALESCE(failed_login_attempts, 0)::int FROM isahl_auth.auth_users \
         WHERE email = $1 ORDER BY id",
    )
    .bind(&email)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        counters,
        vec![0, 0],
        "候选密码全不匹配 MUST NOT 增加失败计数（防跨账号锁定）"
    );

    // 4. 选定 username 后走单列路径：登录成功并签发令牌
    let req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({ "identifier": user_b, "password": pass_b }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "按所选 username 登录 MUST 成功"
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["access_token"].is_string(), "单列路径 MUST 签发令牌");

    // CLEANUP
    common::cleanup_user_by_email(&pool, &email).await.ok();
}
