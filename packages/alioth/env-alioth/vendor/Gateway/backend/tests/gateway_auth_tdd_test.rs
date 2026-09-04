//! Gateway 用户认证与授权 TDD 验证测试
//!
//! 测试 Gateway 作为业务应用统一入口的认证代理和授权执行：
//!   1. Gateway 健康检查 / CORS
//!   2. Gateway 认证代理路由配置
//!   3. PEP (Policy Enforcement Point) JWT 验证
//!   4. 受保护 API 的认证守卫
//!
//! TDD 策略：先编写测试定义预期行为，再验证实现。

mod common;

use ::common::RateLimitMiddleware;
use actix_web::{test, web, App, HttpResponse};
use serde_json::json;

/// 测试用 EC P-256 私钥（PKCS#8 PEM），与 `NgacEnforcer::new_without_pool()` 中的公钥配对。
const TEST_SSO_JWT_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs
\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0
\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm
\
-----END PRIVATE KEY-----";

/// 与验证公钥不匹配的 EC P-256 测试私钥，用于签名验证失败场景。
const TEST_SSO_JWT_PRIVATE_KEY_WRONG: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgR9vYorKu4Jx4T+dE
\
Bxe8wC8o84bQTYnuyR8PvAetVZGhRANCAATSycOwadUbFvVHZNs+p8NLeLxEeRwz
\
X1MW6TgH1PjYp1JiKOF/NbW0q8ZRnr0EaBeRAR8ozzOFX0vIo6nUcrpA
\
-----END PRIVATE KEY-----";

// ============================================================================
// 测试组 1: Gateway 基础健康检查
// ============================================================================

/// TDD-GW-001: Gateway 健康检查端点
///
/// Given:  Gateway 服务启动
/// When:   GET /health
/// Then:   返回 200 OK 含 status: "ok"
#[actix_web::test]
async fn tdd_gw_001_health_check_returns_ok() {
    async fn health() -> HttpResponse {
        HttpResponse::Ok().json(json!({"status": "ok"}))
    }

    let app = test::init_service(App::new().route("/health", web::get().to(health))).await;

    let req = test::TestRequest::get().uri("/health").to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "ok");
}

/// TDD-GW-002: Gateway 404 返回 JSON 错误格式
///
/// Given:  一个不存在的路由
/// When:   GET /nonexistent
/// Then:   返回 404 JSON，包含 error 字段
#[actix_web::test]
async fn tdd_gw_002_not_found_returns_json() {
    async fn not_found() -> HttpResponse {
        HttpResponse::NotFound().json(json!({
            "error": "not_found",
            "message": "The requested resource does not exist"
        }))
    }

    let app = test::init_service(App::new().default_service(web::route().to(not_found))).await;

    let req = test::TestRequest::get()
        .uri("/some-non-existent-path")
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status().as_u16(), 404);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "not_found");
    assert!(body["message"].is_string());
}

// ============================================================================
// 测试组 2: Gateway 认证代理路由
// ============================================================================

/// TDD-GW-003: Gateway 配置了 /auth 代理路由
///
/// Given:  Gateway 路由配置
/// When:   POST /auth/register
/// Then:   路由被正确注册（请求被转发处理，而非 404）
///
/// 注意：此测试验证路由存在性，不依赖 SSO 服务实际运行。
#[actix_web::test]
async fn tdd_gw_003_auth_proxy_route_registered() {
    // 模拟 SSO 代理处理函数
    async fn mock_sso_proxy() -> HttpResponse {
        // 在真实 Gateway 中，这会将请求转发到 SSO 服务
        // 这里只验证路由被注册
        HttpResponse::Ok().json(json!({"proxied": true}))
    }

    let app = test::init_service(
        App::new()
            .service(web::scope("/auth").route("/{tail:.*}", web::route().to(mock_sso_proxy)))
            .service(web::scope("/api/auth").route("/{tail:.*}", web::route().to(mock_sso_proxy))),
    )
    .await;

    // 验证 /auth/register 路由
    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({"email": "test@test.com", "password": "Test123456"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["proxied"], true, "/auth/* should be proxied to SSO");

    // 验证 /auth/login 路由
    let req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({"identifier": "test@test.com", "password": "Test123456"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);

    // 验证 /api/auth/login 别名路由
    let req = test::TestRequest::post()
        .uri("/api/auth/login")
        .set_json(json!({"identifier": "test@test.com", "password": "Test123456"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);

    // 验证 /auth/logout 路由
    let req = test::TestRequest::post().uri("/auth/logout").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
}

/// TDD-GW-004: Gateway CORS 配置
///
/// Given:  Gateway 配置了 CORS
/// When:   发送 OPTIONS 预检请求
/// Then:   返回允许的 CORS 头
#[actix_web::test]
async fn tdd_gw_004_cors_preflight_returns_headers() {
    use actix_cors::Cors;
    use actix_web::http::header;

    let cors = Cors::default()
        .allowed_origin_fn(|origin, _req_head| {
            let o = origin.to_str().unwrap_or("");
            o.starts_with("http://localhost:")
        })
        .allowed_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
        .allowed_headers(vec![
            header::AUTHORIZATION,
            header::ACCEPT,
            header::CONTENT_TYPE,
        ])
        .supports_credentials()
        .max_age(3600);

    async fn health() -> HttpResponse {
        HttpResponse::Ok().json(json!({"status": "ok"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(cors)
            .route("/health", web::get().to(health)),
    )
    .await;

    let req = test::TestRequest::default()
        .method(actix_web::http::Method::OPTIONS)
        .uri("/health")
        .insert_header(("Origin", "http://localhost:13000"))
        .insert_header(("Access-Control-Request-Method", "GET"))
        .insert_header(("Access-Control-Request-Headers", "Content-Type"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "CORS preflight should succeed");

    let allow_origin = resp.headers().get("access-control-allow-origin");
    assert!(
        allow_origin.is_some(),
        "CORS preflight must include Access-Control-Allow-Origin"
    );
}

// ============================================================================
// 测试组 3: PEP JWT 验证
// ============================================================================

/// TDD-GW-005: 无 Authorization 头的请求被拒绝
///
/// Given:  PEP 中间件保护的 API
/// When:   不带 Authorization 头发送请求
/// Then:   返回 401 Unauthorized
#[actix_web::test]
async fn tdd_gw_005_missing_auth_header_returns_401() {
    use alioth_gateway::pep::NgacEnforcer;

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    let req = test::TestRequest::get().uri("/api/protected").to_request();
    let resp = test::call_service(&app, req).await;

    // 无认证令牌应返回 401
    assert_eq!(
        resp.status().as_u16(),
        401,
        "Missing Authorization header should return 401"
    );
}

/// TDD-GW-006: 无效 JWT 被拒绝
///
/// Given:  PEP 中间件保护的 API
/// When:   携带无效 JWT 令牌
/// Then:   返回 401 Unauthorized
#[actix_web::test]
async fn tdd_gw_006_invalid_jwt_returns_401() {
    use alioth_gateway::pep::NgacEnforcer;

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/protected")
        .insert_header(("Authorization", "Bearer invalid-token-not-real-jwt"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status().as_u16(), 401, "Invalid JWT should return 401");
}

/// TDD-GW-007: 格式错误的 Authorization 头被拒绝
///
/// Given:  PEP 中间件保护的 API
/// When:   Authorization 头格式不正确（非 Bearer）
/// Then:   返回 401 Unauthorized
#[actix_web::test]
async fn tdd_gw_007_malformed_auth_header_returns_401() {
    use alioth_gateway::pep::NgacEnforcer;

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    // 测试各种格式错误的 Authorization 头
    let invalid_headers = vec![
        "Basic dXNlcjpwYXNz", // Basic auth instead of Bearer
        "Bearer",             // Bearer without token
        "",                   // Empty
        "not-even-a-valid-format",
    ];

    for header_value in &invalid_headers {
        let req = test::TestRequest::get()
            .uri("/api/protected")
            .insert_header(("Authorization", *header_value))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status().as_u16(),
            401,
            "Malformed auth header '{}' should return 401",
            header_value
        );
    }
}

/// TDD-GW-008: 有效 JWT 通过 PEP 验证
///
/// Given:  PEP 中间件保护的 API
/// When:   携带由 SSO 颁发的有效 JWT
/// Then:   请求通过，返回 200（PDP 可能不可用，但 JWT 验证应通过）
///
/// 注意：此测试使用有效的 JWT 签名和声明，
/// 但由于没有 SSO PDP 可用，实际结果取决于 PDP 超时策略。
/// 核心验证：JWT 签名验证通过，不会因 token 格式问题被拒绝。
#[actix_web::test]
async fn tdd_gw_008_valid_jwt_passes_pep_validation() {
    use alioth_gateway::pep::NgacEnforcer;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    #[derive(serde::Serialize)]
    struct TestClaims {
        sub: String,
        email: String,
        username: String,
        exp: usize,
        iat: usize,
        // PEP 显式强制 iss/aud 绑定（缺声明 → 401），有效令牌必须携带
        // 与默认绑定（http://localhost:9002）一致的 iss/aud。
        iss: String,
        aud: String,
    }

    let private_key = TEST_SSO_JWT_PRIVATE_KEY;

    // 生成有效的 JWT
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let claims = TestClaims {
        sub: "test-user-001".to_string(),
        email: "test@alioth.test".to_string(),
        username: "test_user".to_string(),
        exp: now + 3600, // 1 hour from now
        iat: now,
        iss: "http://localhost:9002".to_string(),
        aud: "http://localhost:9002".to_string(),
    };

    let token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(private_key).unwrap(),
    )
    .expect("JWT encoding should succeed");

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/protected")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;

    // 由于没有 SSO PDP 可用，PDP 检查可能超时或失败
    // 但 JWT 签名验证本身应通过
    // 实际状态码取决于 PDP 错误处理策略
    let status = resp.status().as_u16();
    eprintln!("PEP response with valid JWT (no PDP): {}", status);

    // JWT 验证通过的表征：不应返回 401（那是 token 无效）
    // PDP 不可用时可能返回 403（禁止）或 502/503（网关错误）
    assert_ne!(
        status, 401,
        "Valid JWT should not be rejected as unauthorized. Got status: {}",
        status
    );
}

/// TDD-GW-009: 过期 JWT 被拒绝
///
/// Given:  PEP 中间件保护的 API
/// When:   携带已过期的 JWT
/// Then:   返回 401 Unauthorized
#[actix_web::test]
async fn tdd_gw_009_expired_jwt_returns_401() {
    use alioth_gateway::pep::NgacEnforcer;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    #[derive(serde::Serialize)]
    struct TestClaims {
        sub: String,
        email: String,
        username: String,
        exp: usize,
        iat: usize,
    }

    let private_key = TEST_SSO_JWT_PRIVATE_KEY;

    // 生成已过期的 JWT（1 小时前过期）
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let claims = TestClaims {
        sub: "test-user-001".to_string(),
        email: "test@alioth.test".to_string(),
        username: "test_user".to_string(),
        exp: now - 3600, // expired 1 hour ago
        iat: now - 7200,
    };

    let token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(private_key).unwrap(),
    )
    .expect("JWT encoding should succeed");

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/protected")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status().as_u16(), 401, "Expired JWT should return 401");
}

// ============================================================================
// 测试组 4: JWT 令牌签名验证
// ============================================================================

/// TDD-GW-010: 错误密钥签名的 JWT 被拒绝
///
/// Given:  PEP 中间件使用正确密钥
/// When:   使用不同密钥签名的 JWT
/// Then:   返回 401 Unauthorized
#[actix_web::test]
async fn tdd_gw_010_wrong_signature_jwt_returns_401() {
    use alioth_gateway::pep::NgacEnforcer;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    #[derive(serde::Serialize)]
    struct TestClaims {
        sub: String,
        email: String,
        username: String,
        exp: usize,
        iat: usize,
    }

    // 使用不同的密钥签名 JWT
    let wrong_private_key = TEST_SSO_JWT_PRIVATE_KEY_WRONG;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let claims = TestClaims {
        sub: "wrong-sig-test".to_string(),
        email: "wrong@alioth.test".to_string(),
        username: "wrong_sig".to_string(),
        exp: now + 3600,
        iat: now,
    };

    let token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(wrong_private_key).unwrap(),
    )
    .expect("JWT encode with wrong key");

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/protected")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(
        resp.status().as_u16(),
        401,
        "Wrong-signature JWT should return 401"
    );
}

// ============================================================================
// 测试组 5: Rate Limit 限流
// ============================================================================

/// TDD-GW-011: 注册端点限流 — 超过容量返回 429
///
/// Given:  /auth/register 限流容量 = 2，不补充
/// When:   连续发送 3 个请求
/// Then:   前 2 个返回 200，第 3 个返回 429 Too Many Requests
#[actix_web::test]
async fn tdd_gw_011_register_rate_limit_returns_429() {
    async fn mock_register() -> HttpResponse {
        HttpResponse::Ok().json(json!({"success": true}))
    }

    let app = test::init_service(
        App::new()
            .wrap(RateLimitMiddleware::per_ip_any(
                &["/auth/register", "/api/auth/register"],
                2.0, // capacity: 2 requests
                0.0, // refill_rate: 0 (no refill during test)
            ))
            .route("/auth/register", web::post().to(mock_register)),
    )
    .await;

    // Request 1: should pass
    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({"email": "test1@example.com", "password": "Test123456"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "First request should pass");

    // Request 2: should pass
    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({"email": "test2@example.com", "password": "Test123456"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "Second request should pass");

    // Request 3: should be rate limited (429)
    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({"email": "test3@example.com", "password": "Test123456"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        429,
        "Third request should be rate limited (429)"
    );

    // Verify 429 response body contains error information
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body.get("error").is_some() || body.get("message").is_some(),
        "Rate limit response should contain error information"
    );
}

/// TDD-GW-012: 限流器按 IP 隔离
///
/// Given:  /auth/login 限流容量 = 1，不补充
/// When:   不同 IP 地址各发送 1 个请求
/// Then:   所有请求都返回 200（互不干扰）
#[actix_web::test]
async fn tdd_gw_012_rate_limit_isolates_by_ip() {
    async fn mock_login() -> HttpResponse {
        HttpResponse::Ok().json(json!({"token": "fake-jwt"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(RateLimitMiddleware::per_ip_any(
                &["/auth/login", "/api/auth/login"],
                1.0, // capacity: 1 request per IP
                0.0, // no refill
            ))
            .route("/auth/login", web::post().to(mock_login)),
    )
    .await;

    // Request from IP 1: should pass
    let req = test::TestRequest::post()
        .uri("/auth/login")
        .insert_header(("X-Forwarded-For", "192.168.1.10"))
        .set_json(json!({"identifier": "user1", "password": "pass1"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "Request from IP 1 should pass");

    // Request from IP 2: should also pass (different bucket)
    let req = test::TestRequest::post()
        .uri("/auth/login")
        .insert_header(("X-Forwarded-For", "192.168.1.20"))
        .set_json(json!({"identifier": "user2", "password": "pass2"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "Request from IP 2 should pass");

    // Second request from IP 1: should be rate limited
    let req = test::TestRequest::post()
        .uri("/auth/login")
        .insert_header(("X-Forwarded-For", "192.168.1.10"))
        .set_json(json!({"identifier": "user1", "password": "pass1"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        429,
        "Second request from IP 1 should be rate limited"
    );
}

/// TDD-GW-013: 限流路径前缀匹配
///
/// Given:  /auth/register 限流，/auth/login 不限流
/// When:   向 /auth/login 发送大量请求
/// Then:   /auth/login 不受 /auth/register 的限流影响
#[actix_web::test]
async fn tdd_gw_013_rate_limit_prefix_isolation() {
    async fn mock_login() -> HttpResponse {
        HttpResponse::Ok().json(json!({"token": "fake-jwt"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(RateLimitMiddleware::per_ip_any(
                &["/auth/register"],
                1.0, // only 1 request allowed on /auth/register
                0.0,
            ))
            .route("/auth/login", web::post().to(mock_login)),
    )
    .await;

    // Send 3 requests to /auth/login — none should be rate limited
    for i in 0..3 {
        let req = test::TestRequest::post()
            .uri("/auth/login")
            .set_json(json!({"identifier": format!("user{}", i), "password": "pass"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status().as_u16(),
            200,
            "/auth/login should not be affected by /auth/register rate limit"
        );
    }
}

// ============================================================================
// 测试组 6: 邮箱认证
// ============================================================================

/// TDD-GW-014: 邮箱验证码发送端点返回成功
///
/// Given:  /auth/email/send-code 路由
/// When:   POST 请求携带有效邮箱
/// Then:   返回 200 且 sent = true
#[actix_web::test]
async fn tdd_gw_014_email_send_code_returns_success() {
    async fn mock_send_code() -> HttpResponse {
        HttpResponse::Ok().json(json!({"sent": true, "expires_in_minutes": 15}))
    }

    let app = test::init_service(
        App::new().route("/auth/email/send-code", web::post().to(mock_send_code)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/email/send-code")
        .set_json(json!({"email": "test@example.com", "purpose": "register"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["sent"], true);
}

/// TDD-GW-015: 邮箱验证码端点限流
///
/// Given:  /auth/email/send-code 限流容量 = 2，不补充
/// When:   连续发送 3 个请求
/// Then:   前 2 个返回 200，第 3 个返回 429
#[actix_web::test]
async fn tdd_gw_015_email_send_code_rate_limited() {
    async fn mock_send_code() -> HttpResponse {
        HttpResponse::Ok().json(json!({"sent": true}))
    }

    let app = test::init_service(
        App::new()
            .wrap(RateLimitMiddleware::per_ip_any(
                &["/auth/email/send-code"],
                2.0,
                0.0,
            ))
            .route("/auth/email/send-code", web::post().to(mock_send_code)),
    )
    .await;

    for _ in 0..2 {
        let req = test::TestRequest::post()
            .uri("/auth/email/send-code")
            .set_json(json!({"email": "test@example.com"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 200);
    }

    let req = test::TestRequest::post()
        .uri("/auth/email/send-code")
        .set_json(json!({"email": "test@example.com"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 429);
}

// ============================================================================
// 测试组 7: Gateway Workspace 公开路径白名单
// ============================================================================

/// TDD-GW-016: 白名单路径在有效 JWT 下跳过 NGAC PDP
///
/// Given:  /api/global/overview 与 /api/schedule/overview 在白名单中
/// When:   携带有效 JWT 请求
/// Then:   返回 200，不触发 PDP 检查
#[actix_web::test]
async fn tdd_gw_016_public_workspace_paths_skip_ngac_with_valid_jwt() {
    use alioth_gateway::pep::NgacEnforcer;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use std::collections::HashSet;

    #[derive(serde::Serialize)]
    struct TestClaims {
        sub: String,
        email: String,
        username: String,
        exp: usize,
        iat: usize,
        iss: String,
        aud: String,
    }

    let private_key = TEST_SSO_JWT_PRIVATE_KEY;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let claims = TestClaims {
        sub: "42".to_string(),
        email: "workspace@alioth.test".to_string(),
        username: "workspace_user".to_string(),
        exp: now + 3600,
        iat: now,
        iss: "http://localhost:9002".to_string(),
        aud: "http://localhost:9002".to_string(),
    };

    let token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(private_key).unwrap(),
    )
    .expect("JWT encoding should succeed");

    async fn workspace_handler() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "workspace"}))
    }

    let public_paths: HashSet<String> = [
        "/api/global/overview".to_string(),
        "/api/schedule/overview".to_string(),
    ]
    .into();

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool().with_public_noauth_paths(public_paths))
            .route("/api/global/overview", web::get().to(workspace_handler))
            .route("/api/schedule/overview", web::get().to(workspace_handler)),
    )
    .await;

    for path in ["/api/global/overview", "/api/schedule/overview"] {
        let req = test::TestRequest::get()
            .uri(path)
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status().as_u16(),
            200,
            "Public workspace path {} should be accessible with valid JWT",
            path
        );
    }
}

/// TDD-GW-017: noauth 白名单路径完全公开（无 JWT 放行）
///
/// Given:  /api/global/overview 在 public_noauth_paths 中
/// When:   不带任何认证信息请求
/// Then:   返回 200（noauth 路径跳过所有认证——login/PDP 回调等端点语义；
///         「需 JWT 但跳过 PDP」白名单已由 remove-public-whitelist 移除）
#[actix_web::test]
async fn tdd_gw_017_noauth_path_fully_public() {
    use alioth_gateway::pep::NgacEnforcer;
    use std::collections::HashSet;

    async fn workspace_handler() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "workspace"}))
    }

    let public_paths: HashSet<String> = ["/api/global/overview".to_string()].into();

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool().with_public_noauth_paths(public_paths))
            .route("/api/global/overview", web::get().to(workspace_handler)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/global/overview")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "Noauth path should be fully public (no JWT required)"
    );
}

/// TDD-GW-018: 非白名单路径仍受 NGAC PDP 约束
///
/// Given:  /api/protected 不在白名单中且未配置 PDP
/// When:   携带有效 JWT 请求
/// Then:   返回 403 Forbidden（PDP 默认拒绝）
#[actix_web::test]
async fn tdd_gw_018_non_public_path_still_enforces_ngac() {
    use alioth_gateway::pep::NgacEnforcer;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use std::collections::HashSet;

    // 固定 fail-close：dev 环境 .mise.toml 默认 NGAC_FAIL_OPEN=true（fail-open
    // 跳过 PDP），会使本用例返回 200 而断言失败。pin 后与运行环境解耦。
    std::env::set_var("NGAC_FAIL_OPEN", "false");

    #[derive(serde::Serialize)]
    struct TestClaims {
        sub: String,
        email: String,
        username: String,
        exp: usize,
        iat: usize,
        iss: String,
        aud: String,
    }

    let private_key = TEST_SSO_JWT_PRIVATE_KEY;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let claims = TestClaims {
        sub: "42".to_string(),
        email: "workspace@alioth.test".to_string(),
        username: "workspace_user".to_string(),
        exp: now + 3600,
        iat: now,
        iss: "http://localhost:9002".to_string(),
        aud: "http://localhost:9002".to_string(),
    };

    let token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(private_key).unwrap(),
    )
    .expect("JWT encoding should succeed");

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let public_paths: HashSet<String> = ["/api/global/overview".to_string()].into();

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool().with_public_noauth_paths(public_paths))
            .route("/api/global/overview", web::get().to(protected))
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/protected")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();
    assert_ne!(
        status, 200,
        "Non-public path should not bypass NGAC enforcement"
    );
    assert_ne!(
        status, 401,
        "Non-public path with valid JWT should not be rejected as unauthorized"
    );
}

// ============================================================================
// 测试组 5: PEP 安全 header 剥离与注入（fix-pep-rls-column-fail-open）
// ============================================================================

/// 构造带指定 claim 的 ES256 测试 JWT。
fn pep_test_token(sub: &str) -> String {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;
    let claims = serde_json::json!({
        "sub": sub,
        "email": format!("{}@alioth.test", sub),
        "username": format!("user_{}", sub),
        "exp": now + 3600,
        "iat": now,
        "iss": "http://localhost:9002",
        "aud": "http://localhost:9002",
    });
    encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_SSO_JWT_PRIVATE_KEY).unwrap(),
    )
    .expect("JWT encoding should succeed")
}

/// TDD-GW-019: 入站伪造安全 header 被剥离，standalone 注入全量列授权
///
/// Given:  客户端自发携带 x-visible-ids / x-authorized-columns 伪造值
/// When:   携带有效 JWT 请求受保护路径（new_without_pool = 无 PDP standalone）
/// Then:   伪造值不可达业务 handler；x-authorized-columns 被注入 `*`
#[actix_web::test]
async fn tdd_gw_019_inbound_security_headers_stripped_and_wildcard_injected() {
    use alioth_gateway::pep::NgacEnforcer;

    // pin FAIL_OPEN：无 PDP 客户端时放行路径（standalone 语义），与运行环境解耦
    std::env::set_var("NGAC_FAIL_OPEN", "true");

    async fn echo_headers(req: actix_web::HttpRequest) -> HttpResponse {
        let visible = req
            .headers()
            .get("x-visible-ids")
            .map(|v| v.to_str().unwrap_or("").to_string())
            .unwrap_or_else(|| "<missing>".to_string());
        let columns = req
            .headers()
            .get("x-authorized-columns")
            .map(|v| v.to_str().unwrap_or("").to_string())
            .unwrap_or_else(|| "<missing>".to_string());
        HttpResponse::Ok().json(json!({
            "x_visible_ids": visible,
            "x_authorized_columns": columns,
        }))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/echo-headers", web::get().to(echo_headers)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/echo-headers")
        .insert_header(("Authorization", format!("Bearer {}", pep_test_token("19"))))
        .insert_header(("x-visible-ids", "999,888"))
        .insert_header(("x-authorized-columns", "secret_col"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "valid JWT should pass PEP");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["x_visible_ids"], "<missing>",
        "伪造 x-visible-ids 必须被剥离"
    );
    assert_eq!(
        body["x_authorized_columns"], "*",
        "standalone 无 PDP 模式必须注入显式 * 列授权"
    );
}

/// TDD-GW-020: NGAC_FAIL_OPEN=true 分支同样注入全量列授权
///
/// Given:  NGAC_FAIL_OPEN=true（dev/交付逃生门）
/// When:   携带有效 JWT 请求受保护路径
/// Then:   x-authorized-columns 被注入 `*`，伪造值被剥离
#[actix_web::test]
async fn tdd_gw_020_fail_open_injects_wildcard_columns() {
    use alioth_gateway::pep::NgacEnforcer;

    async fn echo_columns(req: actix_web::HttpRequest) -> HttpResponse {
        let columns = req
            .headers()
            .get("x-authorized-columns")
            .map(|v| v.to_str().unwrap_or("").to_string())
            .unwrap_or_else(|| "<missing>".to_string());
        HttpResponse::Ok().json(json!({"x_authorized_columns": columns}))
    }

    std::env::set_var("NGAC_FAIL_OPEN", "true");

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/echo-columns", web::get().to(echo_columns)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/echo-columns")
        .insert_header(("Authorization", format!("Bearer {}", pep_test_token("20"))))
        .insert_header(("x-authorized-columns", "forged"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["x_authorized_columns"], "*",
        "FAIL_OPEN 模式必须注入显式 * 且剥离伪造值"
    );
}

/// TDD-GW-021: cache 命中后第二次请求仍注入权威列授权
///
/// Given:  NGAC_FAIL_OPEN=false，无 PDP 客户端（new_without_pool → 500 拒绝）
/// 无法直接驱动缓存命中；本用例改为验证独立路径：同一 JWT 两次请求，
/// 无 PDP 时均被拒绝（fail-close），不产生漏注入放行。
/// 缓存命中注入逻辑由单元层 + G5 端到端复现验证覆盖。
#[actix_web::test]
async fn tdd_gw_021_no_pdp_fail_close_consistent() {
    use alioth_gateway::pep::NgacEnforcer;

    std::env::set_var("NGAC_FAIL_OPEN", "false");

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    // 无 PDP 客户端且非 FAIL_OPEN → 两个分支都应 fail-close（500/403），不得 200
    for _ in 0..2 {
        let req = test::TestRequest::get()
            .uri("/api/protected")
            .insert_header(("Authorization", format!("Bearer {}", pep_test_token("21"))))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_ne!(
            resp.status().as_u16(),
            200,
            "无 PDP 且 FAIL_OPEN=false 时不得放行"
        );
    }
}

/// TDD-GW-022: 无效 token 在 header 剥离后仍被 JWT 校验拒绝
#[actix_web::test]
async fn tdd_gw_022_invalid_token_rejected_after_strip() {
    use alioth_gateway::pep::NgacEnforcer;

    std::env::set_var("NGAC_FAIL_OPEN", "true");

    async fn protected() -> HttpResponse {
        HttpResponse::Ok().json(json!({"data": "secret"}))
    }

    let app = test::init_service(
        App::new()
            .wrap(NgacEnforcer::new_without_pool())
            .route("/api/protected", web::get().to(protected)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/protected")
        .insert_header(("Authorization", "Bearer invalid.token.here"))
        .insert_header(("x-visible-ids", "1,2,3"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "无效 token 必须 401，伪造 header 不影响认证判定"
    );
}
