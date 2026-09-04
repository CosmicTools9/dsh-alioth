//! WZ OpenAPI 模拟调用方接入集成测试（openapi-wz-mock-client）
//!
//! 以「模拟第三方调用方」身份验证 WZ 开放 API 的完整调用契约：
//!   1. 有效 scope 服务令牌 → 调 WZ 物权端点（/api/service/wz-yy-wms-service/bin-titles）→ 非 401/403
//!   2. 有效 scope → 调 WZ 仓储端点（/api/service/zc-id-stus-inventory-service/inbound-orders）→ 非 401/403
//!   3. scope 不足 → 403 SCOPE_INSUFFICIENT（fail-closed）
//!   4. 无令牌 → 401
//!   5. per-client 限流超限 → 429 + Retry-After/X-RateLimit-*（Free 套餐 1rps 语义）
//!
//! 模拟方式：测试 JWT（sub=client:* + svc_user_id）——与 SSO client_credentials
//! 签发的服务令牌同构（PEP 按 sub 前缀 + svc_user_id 解析）。端点用真实 WZ
//! openapi.json 中的路径注册 mock handler（响应体为 WZ 实体示例）。

use actix_web::{test, web, App, HttpResponse};
use alioth_gateway::pep::NgacEnforcer;
use common::RateLimitMiddleware;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;

/// 测试用 EC P-256 私钥（与 NgacEnforcer::new_without_pool 内置公钥配对；
/// 同 openapi_pep_integration_test.rs）。
const TEST_SSO_JWT_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs
\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0
\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm
\
-----END PRIVATE KEY-----";

/// 签发模拟调用方服务令牌（ES256，sub=client:* + svc_user_id + scope）。
fn issue_token(sub: &str, svc_user_id: i64, scope: &str) -> String {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = serde_json::json!({
        "sub": sub,
        "exp": now + 3600,
        "iat": now,
        "email": "",
        "username": "",
        "sid": "",
        "iss": "http://localhost:9002",
        "aud": "http://localhost:9002",
        "scope": scope,
        "svc_user_id": svc_user_id,
    });
    let header = Header::new(Algorithm::ES256);
    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_SSO_JWT_PRIVATE_KEY).unwrap(),
    )
    .unwrap()
}

/// WZ-OPENAPI-001: 模拟调用方（有效 scope）调 WZ 物权端点
///
/// Given: 服务令牌 sub=client:mock-xingtai, scope=read:bin_titles
/// When:  GET /api/service/wz-yy-wms-service/bin-titles
/// Then:  非 401/403（scope 覆盖；PDP 不可用时 500 为预期——new_without_pool）
#[actix_web::test]
async fn wz_openapi_001_mock_client_bin_titles_ok() {
    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                "http://localhost:9002".to_string(),
                "http://localhost:9002".to_string(),
            ))
            .service(
                web::scope("/api")
                    .route(
                        "/service/wz-yy-wms-service/bin-titles",
                        web::get()
                            .to(|| async { HttpResponse::Ok().json(json!({"success": true})) }),
                    )
                    .route(
                        "/service/zc-id-stus-inventory-service/inbound-orders",
                        web::get()
                            .to(|| async { HttpResponse::Ok().json(json!({"success": true})) }),
                    ),
            ),
    )
    .await;
    let token = issue_token("client:mock-xingtai", 42, "read:bin_titles");
    let req = test::TestRequest::get()
        .uri("/api/service/wz-yy-wms-service/bin-titles")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 500,
        "有效 scope 服务令牌不应 401/403（PDP 不可用时 500 为预期），got {status}"
    );
}

/// WZ-OPENAPI-002: 模拟调用方调 WZ 仓储端点
#[actix_web::test]
async fn wz_openapi_002_mock_client_inbound_orders_ok() {
    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                "http://localhost:9002".to_string(),
                "http://localhost:9002".to_string(),
            ))
            .service(
                web::scope("/api")
                    .route(
                        "/service/wz-yy-wms-service/bin-titles",
                        web::get()
                            .to(|| async { HttpResponse::Ok().json(json!({"success": true})) }),
                    )
                    .route(
                        "/service/zc-id-stus-inventory-service/inbound-orders",
                        web::get()
                            .to(|| async { HttpResponse::Ok().json(json!({"success": true})) }),
                    ),
            ),
    )
    .await;
    let token = issue_token("client:mock-xingtai", 42, "read:inbound_orders");
    let req = test::TestRequest::get()
        .uri("/api/service/zc-id-stus-inventory-service/inbound-orders")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 500,
        "有效 scope 服务令牌不应 401/403，got {status}"
    );
}

/// WZ-OPENAPI-003: scope 不足 → 403 SCOPE_INSUFFICIENT（fail-closed）
#[actix_web::test]
async fn wz_openapi_003_scope_insufficient_rejected() {
    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                "http://localhost:9002".to_string(),
                "http://localhost:9002".to_string(),
            ))
            .service(
                web::scope("/api")
                    .route(
                        "/service/wz-yy-wms-service/bin-titles",
                        web::get()
                            .to(|| async { HttpResponse::Ok().json(json!({"success": true})) }),
                    )
                    .route(
                        "/service/zc-id-stus-inventory-service/inbound-orders",
                        web::get()
                            .to(|| async { HttpResponse::Ok().json(json!({"success": true})) }),
                    ),
            ),
    )
    .await;
    // 令牌 scope=read:inbound_orders，但端点要求 read:bin_titles
    let token = issue_token("client:mock-xingtai", 42, "read:inbound_orders");
    let req = test::TestRequest::get()
        .uri("/api/service/wz-yy-wms-service/bin-titles")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "scope 不足应 403 SCOPE_INSUFFICIENT"
    );
}

/// WZ-OPENAPI-004: 无令牌 → 401
#[actix_web::test]
async fn wz_openapi_004_no_token_unauthorized() {
    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                "http://localhost:9002".to_string(),
                "http://localhost:9002".to_string(),
            ))
            .service(
                web::scope("/api")
                    .route(
                        "/service/wz-yy-wms-service/bin-titles",
                        web::get()
                            .to(|| async { HttpResponse::Ok().json(json!({"success": true})) }),
                    )
                    .route(
                        "/service/zc-id-stus-inventory-service/inbound-orders",
                        web::get()
                            .to(|| async { HttpResponse::Ok().json(json!({"success": true})) }),
                    ),
            ),
    )
    .await;
    let req = test::TestRequest::get()
        .uri("/api/service/wz-yy-wms-service/bin-titles")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401, "无令牌应 401");
}

/// WZ-OPENAPI-005: per-client 限流（Free 套餐 1rps 语义）→ 第 2 个请求 429
#[actix_web::test]
async fn wz_openapi_005_per_client_rate_limit_429() {
    let app = test::init_service(
        App::new()
            .wrap(RateLimitMiddleware::per_client(&["/api/service"], 1.0, 1.0))
            .route(
                "/api/service/wz-yy-wms-service/bin-titles",
                web::get().to(|| async { HttpResponse::Ok().json(json!({"ok": true})) }),
            ),
    )
    .await;
    let token = issue_token("client:mock-xingtai", 42, "read:bin_titles");
    for i in 0..2 {
        let req = test::TestRequest::get()
            .uri("/api/service/wz-yy-wms-service/bin-titles")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        if i == 0 {
            assert_eq!(resp.status().as_u16(), 200, "第 1 个请求应放行");
        } else {
            assert_eq!(resp.status().as_u16(), 429, "第 2 个请求应 429");
            let headers = resp.headers();
            assert!(headers.contains_key("Retry-After"), "缺少 Retry-After");
            assert!(
                headers.contains_key("X-RateLimit-Limit"),
                "缺少 X-RateLimit-Limit"
            );
            assert!(
                headers.contains_key("X-RateLimit-Remaining"),
                "缺少 X-RateLimit-Remaining"
            );
            assert!(
                headers.contains_key("X-RateLimit-Reset"),
                "缺少 X-RateLimit-Reset"
            );
        }
    }
}
