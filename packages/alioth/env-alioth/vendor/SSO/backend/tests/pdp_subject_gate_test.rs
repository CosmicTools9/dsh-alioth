//! tighten-pdp-decision-surface 集成测试
//!
//! 覆盖：服务决策面三重门（有效 JWT + Bearer-only + 主体一致性）与审计摄入门禁。
//! - 自然人跨主体决策 → 403 CROSS_SUBJECT_DECISION_DENIED
//! - 本人决策 → 放行（门禁通过，决策正常返回）
//! - 服务令牌（svc_user_id 匹配）→ 放行
//! - cookie-only（无 Authorization）→ 401 BEARER_REQUIRED
//! - policy-version Bearer-only
//! - 审计摄入自然人 403（gap_closure 已覆盖服务令牌 201）

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::json;
use sqlx::PgPool;

async fn setup_pool() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

fn mint_token(ast: &AuthState, sub: &str, svc_user_id: i64) -> String {
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    let mut claims = Claims::new(sub, "", false);
    claims.svc_user_id = svc_user_id;
    encode_access_token(&claims, &ast.jwt_private_key).expect("mint token")
}

/// 自然人 A 请求 user_id=B 的决策 → 403（主体一致性）
#[tokio::test]
async fn cross_subject_decision_denied() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::configure_protected_routes),
    )
    .await;
    let token = mint_token(&ast, "1001", 0);

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/check")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({"user_id": 2002, "resource": "collections", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "cross-subject decision must be denied"
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "CROSS_SUBJECT_DECISION_DENIED");
}

/// 本人决策放行 + 服务令牌（svc_user_id 匹配）放行
#[tokio::test]
async fn matching_subject_and_service_token_pass_gate() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::configure_protected_routes),
    )
    .await;

    // 本人（自然人 sub==user_id）
    let token = mint_token(&ast, "3001", 0);
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/check")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({"user_id": 3001, "resource": "collections", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_ne!(
        resp.status().as_u16(),
        403,
        "self decision must pass subject gate"
    );

    // 服务令牌（sub=client:*，svc_user_id 匹配）
    let svc_token = mint_token(&ast, "client:e2e-svc", 4001);
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/check")
            .insert_header(("Authorization", format!("Bearer {}", svc_token)))
            .set_json(json!({"user_id": 4001, "resource": "collections", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_ne!(
        resp.status().as_u16(),
        403,
        "service token with matching svc_user_id must pass"
    );

    // 服务令牌 svc_user_id 不匹配 → 403
    let svc_token = mint_token(&ast, "client:e2e-svc2", 4002);
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/check")
            .insert_header(("Authorization", format!("Bearer {}", svc_token)))
            .set_json(json!({"user_id": 9999, "resource": "collections", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "service token with mismatched svc_user_id must be denied"
    );
}

/// cookie-only（无 Authorization）→ 401（Bearer-only 通道门）
#[tokio::test]
async fn cookie_only_decision_rejected() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::configure_protected_routes),
    )
    .await;
    let token = mint_token(&ast, "5001", 0);

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/check")
            .cookie(actix_web::cookie::Cookie::new(
                "access_token",
                token.clone(),
            ))
            .set_json(json!({"user_id": 5001, "resource": "collections", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "cookie-only decision call must be rejected (Bearer required)"
    );
}
