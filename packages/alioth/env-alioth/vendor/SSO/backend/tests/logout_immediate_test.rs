//! fix-sso-noauth-removal-immediate-revoke 集成测试
//!
//! 覆盖：
//! - refresh 会话活性门禁：会话 revoked 后 refresh 拒绝（登出后无 TTL 残留）
//! - logout Bearer-only 撤销：无 cookie 经 Authorization 登出 → 会话 + refresh tokens 全吊销
//! - /api/ngac 无凭证 401（noauth 白名单移除）
//!
//! 注：运行需共享测试库（aliothstudio_test），表结构同 sso_gaps_fix_test。

mod common;

use actix_web::{test, web, App};
use serde_json::json;
use sqlx::PgPool;

async fn setup_pool() -> PgPool {
    ::common::testing::connect_test_db().await
}

// ── refresh 会话活性门禁 ───────────────────────────────────────────────────────

#[tokio::test]
async fn refresh_rejected_after_session_revoked() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();
    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::login::configure)),
    )
    .await;

    // fixture：预验证邮箱 → register → login（token 绑 sid）
    let email = format!("logout-r-{}@test.local", uuid::Uuid::new_v4().simple());
    let username = format!("logout_r_{}", uuid::Uuid::new_v4().simple());
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.auth_email_verifications (email, code, purpose, expires_at, verified) \
         VALUES ($1, '000000', 'register', NOW() + INTERVAL '1 hour', TRUE)",
    )
    .bind(&email)
    .execute(&pool)
    .await;
    let register_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({ "email": email, "username": username, "password": "TestPass123!" }))
        .to_request();
    let resp = test::call_service(&app, register_req).await;
    assert!(
        resp.status().is_success(),
        "fixture register: {:?}",
        resp.status()
    );
    // 注册自动审批流：用户落 pending_approval——激活后登录（tdd_reg_009 同款）
    let _ = sqlx::query("UPDATE isahl_auth.auth_users SET status = 'active' WHERE email = $1")
        .bind(&email)
        .execute(&pool)
        .await;
    let login_req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({ "identifier": email, "password": "TestPass123!" }))
        .to_request();
    let resp = test::call_service(&app, login_req).await;
    assert_eq!(resp.status().as_u16(), 200, "fixture login");
    let access = resp
        .response()
        .cookies()
        .find(|c| c.name() == "access_token")
        .map(|c| c.value().to_string())
        .expect("access_token cookie");
    let refresh = resp
        .response()
        .cookies()
        .find(|c| c.name() == "refresh_token")
        .map(|c| c.value().to_string())
        .expect("refresh_token cookie");

    // 正常刷新应成功（会话 active）
    let ok = test::TestRequest::post()
        .uri("/auth/refresh")
        .insert_header(("Cookie", format!("refresh_token={}", refresh)))
        .to_request();
    let resp = test::call_service(&app, ok).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "refresh with active session should succeed"
    );

    // 直接吊销会话（管理端语义）
    let sid: String = {
        let claims =
            gateway_sso::auth::jwt::decode_token_any(&access, &auth_state.verification_keys())
                .expect("decode access");
        claims.sid
    };
    assert!(!sid.is_empty(), "login token must carry sid");
    sqlx::query("UPDATE isahl_auth.sso_sessions SET status = 'revoked' WHERE session_token = $1")
        .bind(&sid)
        .execute(&pool)
        .await
        .expect("revoke session");

    // 会话吊销后 refresh 必须拒绝（登出后无 TTL 残留）
    let bad = test::TestRequest::post()
        .uri("/auth/refresh")
        .insert_header(("Cookie", format!("refresh_token={}", refresh)))
        .to_request();
    let resp = test::call_service(&app, bad).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "refresh must be rejected after session revoked"
    );

    // 清理
    sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)")
        .bind(&email).execute(&pool).await.ok();
    sqlx::query("DELETE FROM isahl_auth.refresh_tokens WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)")
        .bind(&email).execute(&pool).await.ok();
    sqlx::query("DELETE FROM isahl_auth.auth_email_verifications WHERE email = $1")
        .bind(&email)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email = $1")
        .bind(&email)
        .execute(&pool)
        .await
        .ok();
}

// ── logout Bearer-only 撤销 ────────────────────────────────────────────────────

#[tokio::test]
async fn logout_with_bearer_revokes_session_and_tokens() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();
    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::login::configure)),
    )
    .await;

    // fixture：预验证邮箱 → register → login
    let email = format!("logout-b-{}@test.local", uuid::Uuid::new_v4().simple());
    let username = format!("logout_b_{}", uuid::Uuid::new_v4().simple());
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.auth_email_verifications (email, code, purpose, expires_at, verified) \
         VALUES ($1, '000000', 'register', NOW() + INTERVAL '1 hour', TRUE)",
    )
    .bind(&email)
    .execute(&pool)
    .await;
    let register_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({ "email": email, "username": username, "password": "TestPass123!" }))
        .to_request();
    let resp = test::call_service(&app, register_req).await;
    assert!(
        resp.status().is_success(),
        "fixture register: {:?}",
        resp.status()
    );
    // 注册自动审批流：用户落 pending_approval——激活后登录（tdd_reg_009 同款）
    let _ = sqlx::query("UPDATE isahl_auth.auth_users SET status = 'active' WHERE email = $1")
        .bind(&email)
        .execute(&pool)
        .await;
    let login_req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({ "identifier": email, "password": "TestPass123!" }))
        .to_request();
    let resp = test::call_service(&app, login_req).await;
    assert_eq!(resp.status().as_u16(), 200, "fixture login");
    let access = resp
        .response()
        .cookies()
        .find(|c| c.name() == "access_token")
        .map(|c| c.value().to_string())
        .expect("access_token cookie");

    let sid: String = {
        let claims =
            gateway_sso::auth::jwt::decode_token_any(&access, &auth_state.verification_keys())
                .expect("decode access");
        claims.sid
    };
    let user_id: i64 =
        sqlx::query_scalar("SELECT user_id FROM isahl_auth.sso_sessions WHERE session_token = $1")
            .bind(&sid)
            .fetch_one(&pool)
            .await
            .expect("session user");

    // Bearer-only logout（无任何 cookie）
    let logout_req = test::TestRequest::post()
        .uri("/auth/logout")
        .insert_header(("Authorization", format!("Bearer {}", access)))
        .to_request();
    let resp = test::call_service(&app, logout_req).await;
    assert_eq!(resp.status().as_u16(), 200, "logout should succeed");

    // 会话已 revoke + refresh tokens 全吊销
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.sso_sessions WHERE session_token = $1")
            .bind(&sid)
            .fetch_one(&pool)
            .await
            .expect("session row");
    assert_eq!(
        status, "revoked",
        "session must be revoked by Bearer logout"
    );
    let revoked_cnt: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.refresh_tokens WHERE user_id = $1 AND revoked = FALSE",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(revoked_cnt, 0, "all refresh tokens must be revoked");

    // 清理
    sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.refresh_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_email_verifications WHERE email = $1")
        .bind(&email)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .ok();
}

// ── /api/ngac noauth 白名单移除 ───────────────────────────────────────────────

#[tokio::test]
async fn ngac_endpoints_require_jwt() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();
    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .configure(gateway_sso::configure_protected_routes),
    )
    .await;

    // 无凭证 → 401（RequireAuth scope wrap）
    let req = test::TestRequest::post()
        .uri("/api/ngac/pdp/decide")
        .set_json(json!({"user_id": 1, "resource": "x:0", "action": "read"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "/api/ngac must require JWT (no whitelist)"
    );

    // 无效凭证 → 401
    let req = test::TestRequest::post()
        .uri("/api/ngac/pdp/decide")
        .insert_header(("Authorization", "Bearer not-a-jwt"))
        .set_json(json!({"user_id": 1, "resource": "x:0", "action": "read"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401, "invalid JWT must be rejected");
}
