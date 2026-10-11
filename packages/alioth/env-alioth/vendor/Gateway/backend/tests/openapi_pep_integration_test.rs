//! Gateway PEP 服务令牌（openapi-external-access）集成测试
//!
//! 覆盖：
//!   1. 服务令牌解析：sub=client:* + svc_user_id → 以服务用户走 PDP
//!   2. scope 强制：令牌 scope 不覆盖端点所需 scope → 403 SCOPE_INSUFFICIENT
//!   3. 白名单跳过：服务令牌命中 with_public_noauth_paths 也走 PDP（Deny → 403）
//!   4. fail-closed：svc_user_id=0（旧令牌）不误触发 scope 校验
//!   5. 文档端点须认证（PEP-OPENAPI-009）
//!
//! （限流用例已迁 `openapi_metering_integration_test` METERING-010/011——唯一实现 =
//! metering per-plan 桶；见 change fix-openapi-review-findings §2）
//!
//! 无 DB 依赖（NgacEnforcer::new_without_pool：静态测试公钥验签、无 NGAC
//! client——PDP 不可用时返回 500，用于验证服务令牌不 401/403 且 scope
//! 校验独立于 PDP）。

use actix_web::{test, web, App, HttpResponse};
use alioth_gateway::pep::NgacEnforcer;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;

/// 测试用 EC P-256 私钥（与 NgacEnforcer::new_without_pool 内置公钥配对；
/// 同 gateway_auth_tdd_test.rs 的 TEST_SSO_JWT_PRIVATE_KEY）。
const TEST_SSO_JWT_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs
\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0
\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm
\
-----END PRIVATE KEY-----";

/// 签发测试 JWT（ES256，含 svc_user_id / scope）。
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

// ============================================================================
// 测试组 1: 服务令牌解析与 scope（PEP-OPENAPI-001..004）
// ============================================================================

/// PEP-OPENAPI-001: 服务令牌（sub=client:* + svc_user_id>0）以服务用户走 PDP
///
/// Given: 服务令牌 svc_user_id=42（服务用户存在）
/// When:  GET /api/service/measurement/measurements
/// Then:  不因服务令牌被 401 拒绝；scope 已覆盖不触发 403（进入 PDP 阶段）
#[actix_web::test]
async fn openapi_001_service_token_resolves_to_service_user() {
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                    "http://localhost:9002".to_string(),
                    "http://localhost:9002".to_string(),
                ))
                .route(
                    "/service/measurement/measurements",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"data": []})) }),
                )
                .route(
                    "/service/measurement/measurements/{id}",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"id": 1})) }),
                )
                .route(
                    "/public/path",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"ok": true})) }),
                ),
        ),
    )
    .await;
    let token = issue_token("client:partner-a", 42, "read:measurements");

    let req = test::TestRequest::get()
        .uri("/api/service/measurement/measurements")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_ne!(resp.status().as_u16(), 401, "服务令牌不应被 401 拒绝");
    assert_ne!(resp.status().as_u16(), 403, "scope 已覆盖不应 403");
}

/// PEP-OPENAPI-002: scope 不足 → 403 SCOPE_INSUFFICIENT
///
/// Given: 服务令牌 scope=read:invoice（不含 read:units）
/// When:  GET /api/service/measurement/measurements
/// Then:  返回 403 SCOPE_INSUFFICIENT
#[actix_web::test]
async fn openapi_002_scope_insufficient_rejected() {
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                    "http://localhost:9002".to_string(),
                    "http://localhost:9002".to_string(),
                ))
                .route(
                    "/service/measurement/measurements",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"data": []})) }),
                )
                .route(
                    "/service/measurement/measurements/{id}",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"id": 1})) }),
                )
                .route(
                    "/public/path",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"ok": true})) }),
                ),
        ),
    )
    .await;
    let token = issue_token("client:partner-a", 42, "read:invoice");

    let req = test::TestRequest::get()
        .uri("/api/service/measurement/measurements")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 403, "scope 不足应 403");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "SCOPE_INSUFFICIENT");
}

/// PEP-OPENAPI-003: 通配 scope（*）放行
///
/// Given: 服务令牌 scope=*
/// When:  GET /api/service/measurement/measurements
/// Then:  不因 scope 拒绝（进入 PDP）
#[actix_web::test]
async fn openapi_003_wildcard_scope_permitted() {
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                    "http://localhost:9002".to_string(),
                    "http://localhost:9002".to_string(),
                ))
                .route(
                    "/service/measurement/measurements",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"data": []})) }),
                )
                .route(
                    "/service/measurement/measurements/{id}",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"id": 1})) }),
                )
                .route(
                    "/public/path",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"ok": true})) }),
                ),
        ),
    )
    .await;
    let token = issue_token("client:partner-a", 42, "*");

    let req = test::TestRequest::get()
        .uri("/api/service/measurement/measurements")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_ne!(resp.status().as_u16(), 403, "通配 scope 不应被 403");
}

/// PEP-OPENAPI-004: 自然人令牌不受 scope 校验影响
///
/// Given: 自然人令牌（sub=数字，svc_user_id=0）
/// When:  GET /api/service/measurement/measurements
/// Then:  不触发 scope 校验（走用户 PDP 链路）
#[actix_web::test]
async fn openapi_004_natural_user_not_scope_checked() {
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                    "http://localhost:9002".to_string(),
                    "http://localhost:9002".to_string(),
                ))
                .route(
                    "/service/measurement/measurements",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"data": []})) }),
                )
                .route(
                    "/service/measurement/measurements/{id}",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"id": 1})) }),
                )
                .route(
                    "/public/path",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"ok": true})) }),
                ),
        ),
    )
    .await;
    let token = issue_token("1002", 0, "");

    let req = test::TestRequest::get()
        .uri("/api/service/measurement/measurements")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_ne!(
        resp.status().as_u16(),
        403,
        "自然人令牌不应被 scope 校验拒绝"
    );
}

// ============================================================================
// 测试组 2: 白名单跳过（服务令牌强制 PDP）
// ============================================================================

/// PEP-OPENAPI-005: 服务令牌命中 with_public_noauth_paths 也走 PDP（不跳过）
///
/// Given: 服务令牌请求 /api/public/path（配置在 public_paths 白名单）
/// When:  PEP 处理
/// Then:  服务令牌不命中白名单 → 走 PDP（无 PDP 时 500，而非白名单放行 200）；
///        自然人令牌白名单命中 → 放行 200（现状保持）
#[actix_web::test]
async fn openapi_005_service_token_bypasses_whitelist() {
    let enforcer = NgacEnforcer::new_without_pool()
        .with_token_binding(
            "http://localhost:9002".to_string(),
            "http://localhost:9002".to_string(),
        )
        .with_public_noauth_paths(["/api/public/path".to_string()].into());
    let app = test::init_service(App::new().service(web::scope("/api").wrap(enforcer).route(
        "/public/path",
        web::get().to(|| async { HttpResponse::Ok().json(json!({"ok": true})) }),
    )))
    .await;

    // 服务令牌：即使路径在白名单，也必须走 PDP（无 ngac_client → 500）
    let svc_token = issue_token("client:partner-a", 42, "read:measurements");
    let req = test::TestRequest::get()
        .uri("/api/public/path")
        .insert_header(("Authorization", format!("Bearer {}", svc_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_ne!(resp.status().as_u16(), 200, "服务令牌不得因白名单放行");

    // 自然人令牌：白名单命中 → 放行 200（与现状一致）
    let user_token = issue_token("1002", 0, "");
    let req2 = test::TestRequest::get()
        .uri("/api/public/path")
        .insert_header(("Authorization", format!("Bearer {}", user_token)))
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_eq!(
        resp2.status().as_u16(),
        200,
        "自然人令牌白名单放行（现状保持）"
    );
}

// ============================================================================
// 测试组 3: fail-closed
// ============================================================================

/// PEP-OPENAPI-008: svc_user_id=0 的 client:* 令牌（旧令牌）不误触发 scope 校验
///
/// Given: sub=client:legacy（svc_user_id=0，旧令牌）
/// When:  GET /api/service/measurement/measurements
/// Then:  视为自然人链路（user_id=0），不进入服务令牌 scope 校验
#[actix_web::test]
async fn openapi_008_legacy_client_token_not_misparsed() {
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                    "http://localhost:9002".to_string(),
                    "http://localhost:9002".to_string(),
                ))
                .route(
                    "/service/measurement/measurements",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"data": []})) }),
                )
                .route(
                    "/service/measurement/measurements/{id}",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"id": 1})) }),
                )
                .route(
                    "/public/path",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"ok": true})) }),
                ),
        ),
    )
    .await;
    let token = issue_token("client:legacy", 0, "");

    let req = test::TestRequest::get()
        .uri("/api/service/measurement/measurements")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_ne!(resp.status().as_u16(), 403, "旧令牌不应被 scope 校验拒绝");
}

// ============================================================================
// 测试组 4: 文档端点（L6 补强）
// ============================================================================

/// PEP-OPENAPI-009: 文档端点须认证（匿名 401，无免认证公开）
#[actix_web::test]
async fn openapi_009_doc_endpoint_requires_auth() {
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool().with_token_binding(
                    "http://localhost:9002".to_string(),
                    "http://localhost:9002".to_string(),
                ))
                .route(
                    "/openapi.json",
                    web::get().to(|| async {
                        HttpResponse::Ok().json(json!({"openapi": "3.0.3", "paths": {}}))
                    }),
                ),
        ),
    )
    .await;

    // 匿名访问 openapi.json → 401（无免认证公开端点）
    let anon = test::TestRequest::get()
        .uri("/api/openapi.json")
        .to_request();
    let resp_anon = test::call_service(&app, anon).await;
    assert_eq!(
        resp_anon.status().as_u16(),
        401,
        "匿名访问 openapi.json 应 401"
    );

    // 带服务令牌 → 200（认证通过，文档可读）
    let token = issue_token("client:partner-a", 42, "read:measurements");
    let authed = test::TestRequest::get()
        .uri("/api/openapi.json")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp_authed = test::call_service(&app, authed).await;
    assert_eq!(
        resp_authed.status().as_u16(),
        200,
        "带服务令牌访问 openapi.json 应 200"
    );
}
