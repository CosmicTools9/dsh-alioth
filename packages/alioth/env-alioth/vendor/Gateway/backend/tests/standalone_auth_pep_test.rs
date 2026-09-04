//! Standalone 认证链路集成测试（token 级）
//!
//! 覆盖：
//!   1. standalone 密钥对签发的 token（claims 与 standalone_auth login 同构，
//!      iss=STANDALONE_ISSUER）→ PEP token binding 对齐 STANDALONE_ISSUER → 200
//!   2. 历史死锁回归：PEP 沿用默认 issuer 绑定（http://localhost:9002）→ 401
//!
//! 依赖真实测试库（aliothstudio_test，仅需连接池）。密钥对与
//! src/standalone_auth/mod.rs 的 DEV_PRIVATE_KEY/DEV_PUBLIC_KEY **必须一致**
//! （沿用 gateway_auth_tdd_test.rs 密钥内嵌模式）。

use actix_web::{test, web, App, HttpResponse};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use sqlx::PgPool;

use ::common::testing::connect_test_db;
use alioth_gateway::pep::NgacEnforcer;

/// 与 standalone_auth::STANDALONE_ISSUER 同值（单一事实源在源码模块，此处为测试副本）。
const STANDALONE_ISSUER: &str = "gateway-standalone";

/// standalone_auth/mod.rs DEV_PRIVATE_KEY 的测试副本（必须一致，否则签名校验失败）。
const STANDALONE_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgvTkNZwK8WqNH/aEn
\
rUkSD5+lYAesakhvTFcWpKteHbOhRANCAASmyJF5MqiJ0MkA77TZJkGAdqiqhv26
\
IVcpjkHR5sxTZhZ5eH/SSSV/ddphVgahp0cRM9H4HSgzNMIkDNv5dJuN
\
-----END PRIVATE KEY-----";

/// standalone_auth/mod.rs DEV_PUBLIC_KEY 的测试副本（必须一致，否则签名校验失败）。
const STANDALONE_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEpsiReTKoidDJAO+02SZBgHaoqob9
\
uiFXKY5B0ebMU2YWeXh/0kklf3XaYVYGoadHETPR+B0oMzTCJAzb+XSbjQ==
\
-----END PUBLIC KEY-----";

/// 签发与 standalone login 同构 claims 的 ES256 JWT（iss/aud=STANDALONE_ISSUER，
/// 与 PEP 显式 iss/aud 强制校验对齐——缺失 aud 的令牌现应 401）。
fn issue_standalone_token(sub: &str) -> String {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = json!({
        "sub": sub,
        "email": format!("{}@standalone.local", sub),
        "exp": now + 1800,
        "iat": now,
        "username": sub,
        "namespace": format!("NS-{}", sub),
        "iss": STANDALONE_ISSUER,
        "aud": STANDALONE_ISSUER,
        "sid": "",
    });
    encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(STANDALONE_PRIVATE_KEY).unwrap(),
    )
    .expect("JWT encoding should succeed")
}

/// 构造 PEP：静态公钥 = standalone 公钥，iss/aud 绑定 = 指定 issuer。
/// sso_service_url 传空串（无 PDP 可达）；NGAC_FAIL_OPEN=true 时跳过 PDP 决策。
fn enforcer(pool: &PgPool, issuer: &str) -> NgacEnforcer {
    NgacEnforcer::new(
        pool.clone(),
        STANDALONE_PUBLIC_KEY.to_vec(),
        Vec::new(),
        String::new(),
    )
    .with_token_binding(issuer.to_string(), issuer.to_string())
}

/// TDD-GW-019 同构模式：standalone token → PEP 对齐 STANDALONE_ISSUER → 200
#[actix_web::test]
async fn standalone_chain_001_standalone_token_passes_aligned_pep() {
    std::env::set_var("NGAC_FAIL_OPEN", "true");
    let pool = connect_test_db().await;

    let app = test::init_service(App::new().wrap(enforcer(&pool, STANDALONE_ISSUER)).route(
        "/api/standalone-protected",
        web::get().to(|| async { HttpResponse::Ok().body("ok") }),
    ))
    .await;

    let req = test::TestRequest::get()
        .uri("/api/standalone-protected")
        .insert_header((
            "Authorization",
            format!("Bearer {}", issue_standalone_token("alice1")),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "standalone token（iss=gateway-standalone）应通过对齐 issuer 的 PEP"
    );
}

/// 历史死锁回归：PEP 沿用默认 issuer 绑定（http://localhost:9002）→ 401
#[actix_web::test]
async fn standalone_chain_002_default_issuer_binding_rejects_standalone_token() {
    std::env::set_var("NGAC_FAIL_OPEN", "true");
    let pool = connect_test_db().await;

    let app = test::init_service(
        App::new()
            .wrap(enforcer(&pool, "http://localhost:9002"))
            .route(
                "/api/standalone-protected",
                web::get().to(|| async { HttpResponse::Ok().body("ok") }),
            ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/standalone-protected")
        .insert_header((
            "Authorization",
            format!("Bearer {}", issue_standalone_token("alice2")),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "PEP 默认 issuer 绑定必须拒绝 iss=gateway-standalone 的 token（死锁证据）"
    );
}
