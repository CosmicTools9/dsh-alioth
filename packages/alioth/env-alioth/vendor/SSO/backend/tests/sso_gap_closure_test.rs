//! SSO 缺口闭环测试（sso-gap-closure）
//!
//! 覆盖本次新增端点的成功/否定路径：
//! - H1 社交账号管理：`GET /auth/social/accounts` 与 `DELETE /auth/social/unlink/{providerId}`
//! - H2.3 审计摄入：`POST /api/ngac/audit`
//!
//! 注：运行需已按 `migrations/` 预置的测试库（`user_oauth_accounts`、`identity_providers`、
//! `audit_events` 等表存在）。无 DB 时可 `cargo test --no-run` 仅做编译校验。

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    // 统一走共享测试库连接（含 OS 用户注入），避免 postgres://localhost/... 被
    // sqlx 解析为 anonymous 角色导致连接失败（与 admin_api_test 一致）。
    ::common::testing::connect_test_db().await
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

fn mint_token(ast: &AuthState, user_id: i64, email: &str) -> String {
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    encode_access_token(
        &Claims::new(&user_id.to_string(), email, false),
        &ast.jwt_private_key,
    )
    .expect("mint token")
}

#[actix_web::test]
async fn social_account_list_and_unlink() {
    let pool = connect().await;
    common::setup_schema(&pool).await.ok();

    // 真实用户（user_oauth_accounts.user_id 有 FK → auth_users，不可用假 id）
    let email = format!("social_{}@test.local", uuid::Uuid::new_v4().simple());
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at) \
         VALUES (isahl.gen_next_zuid(), $1, $1, $2, 'active', true, NOW(), NOW()) RETURNING id",
    )
    .bind(&email)
    .bind(&email)
    .fetch_one(&pool)
    .await
    .expect("create test user");
    let provider_id = 99101i64;

    sqlx::query(
        "INSERT INTO isahl_auth.identity_providers (id, name, provider_type) \
         VALUES ($1, 'GitHub', 'oauth') ON CONFLICT (id) DO NOTHING",
    )
    .bind(provider_id)
    .execute(&pool)
    .await
    .ok();

    sqlx::query(
        "DELETE FROM isahl_auth.user_oauth_accounts WHERE user_id = $1 AND provider_id = $2",
    )
    .bind(user_id)
    .bind(provider_id)
    .execute(&pool)
    .await
    .ok();
    sqlx::query(
        "INSERT INTO isahl_auth.user_oauth_accounts \
         (user_id, provider_id, provider_user_id, email, display_name) \
         VALUES ($1, $2, 'gh-123', 'gh@example.com', 'GH User')",
    )
    .bind(user_id)
    .bind(provider_id)
    .execute(&pool)
    .await
    .expect("seed oauth account");

    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::social::configure)),
    )
    .await;

    let token = mint_token(&ast, user_id, "gh@example.com");
    let auth =
        actix_web::http::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap();

    // 列出已关联账号
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/auth/social/accounts")
            .insert_header(("Authorization", auth.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "list social accounts");
    let body: serde_json::Value = test::read_body_json(resp).await;
    let accounts = body["accounts"].as_array().expect("accounts array");
    assert_eq!(accounts.len(), 1, "should list exactly one account");
    assert_eq!(accounts[0]["providerName"].as_str(), Some("GitHub"));
    assert_eq!(accounts[0]["providerType"].as_str(), Some("oauth"));

    // 解绑（成功路径）
    let resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("/auth/social/unlink/{}", provider_id))
            .insert_header(("Authorization", auth.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 204, "unlink social account");

    // 解绑后再列应为空
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/auth/social/accounts")
            .insert_header(("Authorization", auth.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["accounts"].as_array().unwrap().len(),
        0,
        "should be empty after unlink"
    );

    // 否定路径：解绑已不存在的账号应 404
    let resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("/auth/social/unlink/{}", provider_id))
            .insert_header(("Authorization", auth.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 404, "unlink missing account -> 404");

    sqlx::query(
        "DELETE FROM isahl_auth.user_oauth_accounts WHERE user_id = $1 AND provider_id = $2",
    )
    .bind(user_id)
    .bind(provider_id)
    .execute(&pool)
    .await
    .ok();
}

#[actix_web::test]
async fn audit_ingest_endpoint() {
    let pool = connect().await;
    common::setup_schema(&pool).await.ok();

    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/api/ngac").route(
                "/audit",
                web::post().to(gateway_sso::audit::handlers::ingest_event),
            )),
    )
    .await;

    let token = mint_token(&ast, 70201, "audit@example.com");
    let auth =
        actix_web::http::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap();

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/audit")
            .insert_header(("Authorization", auth))
            .set_json(json!({
                "subject_id": 70201,
                "object_type": "document",
                "operation": "read",
                "success": true
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 201, "audit ingest returns 201");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["id"].as_i64().is_some(), "returns event id");
}
