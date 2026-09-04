//! OIDC OP HTTP 全流程集成测试（fix-sso-id-default-heal：sso-gap-report 剩余项闭环）
//!
//! 进程内 HTTP 全流程（此前仅 lib 级单测，无端点装配覆盖）：
//! discovery 200 → 未认证 authorize 401 → Bearer 会话 authorize 302（code+state）
//! → token 兑换 200（id_token）→ JWKS 公钥验签（iss/sub/aud/nonce + exp）
//! → code 重放 400；否定路径：response_type 400 / redirect_uri 越界 400 /
//! client secret 错误 401。

mod common;

use actix_web::http::header::{CONTENT_TYPE, LOCATION};
use actix_web::{test, web, App};
use gateway_sso::auth::client_secret::hash_client_secret;
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::{jwt, AuthState};
use gateway_sso::config::Config;
use sqlx::PgPool;

const ISSUER: &str = "http://localhost:9002";
const RP_REDIRECT: &str = "https://rp.example.com/cb";
const CLIENT_ID: &str = "webapp";

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn test_config() -> Config {
    Config {
        server_addr: "0.0.0.0:9002".into(),
        database_url: String::new(),
        sso_jwt_private_key: String::new(),
        encryption_key: "k".into(),
        ngac_preview_dir: None,
        jwt_access_expiry: 900,
        jwt_refresh_expiry: 604800,
        oauth_google_client_id: None,
        oauth_google_client_secret: None,
        oauth_github_client_id: None,
        oauth_github_client_secret: None,
        oauth_microsoft_client_id: None,
        oauth_microsoft_client_secret: None,
        oauth_microsoft_tenant_id: None,
        oauth_okta_domain: None,
        oauth_okta_client_id: None,
        oauth_okta_client_secret: None,
        oauth_redirect_url: format!("{ISSUER}/auth/callback"),
        oidc_issuer: ISSUER.into(),
        oidc_client_id: None,
        oidc_redirect_uris: vec![RP_REDIRECT.into()],
        log_level: "info".into(),
        identity_verify_mode: "local".into(),
        identity_external_verify_url: None,
        email_mode: "smtp".into(),
        sso_jwt_public_key_prev: None,
    }
}

/// 与 lib 测试同 DDL 的 `oidc_clients` 幂等建表（token 端点 client_secret 校验依赖）。
async fn ensure_oidc_clients_table(pool: &PgPool) {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.oidc_clients (
            id BIGSERIAL PRIMARY KEY,
            client_id VARCHAR(128) NOT NULL UNIQUE,
            client_name VARCHAR(256) NOT NULL DEFAULT '',
            client_secret_hash VARCHAR(256) NOT NULL DEFAULT '',
            redirect_uris TEXT[] NOT NULL DEFAULT '{}',
            enabled BOOLEAN NOT NULL DEFAULT TRUE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            deleted_at TIMESTAMPTZ
        )"#,
    )
    .execute(pool)
    .await
    .expect("ensure oidc_clients");
}

async fn build_deps() -> (PgPool, AuthState) {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("setup");
    ensure_oidc_clients_table(&pool).await;
    // 清理可能残留的测试 client，保证 secret 矩阵确定性
    sqlx::query("DELETE FROM isahl_auth.oidc_clients WHERE client_id IN ('webapp', 'secret-app')")
        .execute(&pool)
        .await
        .ok();
    let ast = common::test_auth_state();
    configure_token_validation(ISSUER.into(), ISSUER.into());
    (pool, ast)
}

macro_rules! init_app {
    ($pool:expr, $ast:expr) => {{
        test::init_service(
            App::new()
                .app_data(web::Data::new($pool.clone()))
                .app_data(web::Data::new($ast.clone()))
                .app_data(web::Data::new(test_config()))
                .route(
                    "/.well-known/openid-configuration",
                    web::get().to(gateway_sso::auth::oidc_provider::oidc_discovery),
                )
                .route(
                    "/oidc/authorize",
                    web::get().to(gateway_sso::auth::oidc_provider::oidc_authorize),
                )
                .route(
                    "/oidc/token",
                    web::post().to(gateway_sso::auth::oidc_provider::oidc_token),
                )
                .route("/.well-known/jwks.json", web::get().to(jwt::jwks)),
        )
        .await
    }};
}

macro_rules! mint {
    ($ast:expr, $user_id:expr) => {{
        encode_access_token(
            &Claims::new(&$user_id.to_string(), "oidc-it@alioth.test", false),
            &$ast.jwt_private_key,
        )
        .expect("mint token")
    }};
}

macro_rules! authorize {
    ($app:expr, $ast:expr, $rt:expr, $cid:expr, $ruri:expr, $st:expr, $nc:expr) => {{
        let token = mint!($ast, 424242);
        test::call_service(
            $app,
            test::TestRequest::get()
                .uri(&format!(
                    "/oidc/authorize?response_type={}&client_id={}\
                     &redirect_uri={}&state={}&nonce={}",
                    $rt, $cid, $ruri, $st, $nc
                ))
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .to_request(),
        )
        .await
    }};
}

#[actix_web::test]
async fn oidc_discovery_document_contract() {
    let (pool, ast) = build_deps().await;
    let app = init_app!(pool, ast);
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/.well-known/openid-configuration")
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());
    let doc: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(doc["issuer"], ISSUER);
    assert_eq!(doc["token_endpoint"], format!("{ISSUER}/oidc/token"));
    assert_eq!(doc["jwks_uri"], format!("{ISSUER}/.well-known/jwks.json"));
    assert!(doc["id_token_signing_alg_values_supported"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "ES256"));
}

#[actix_web::test]
async fn oidc_full_flow_authorize_token_jwks_verify() {
    let (pool, ast) = build_deps().await;
    let app = init_app!(pool, ast);

    // 未认证 → 401
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/oidc/authorize?response_type=code&client_id={CLIENT_ID}&redirect_uri={RP_REDIRECT}"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 401, "未认证 authorize 应 401");

    // Bearer 会话 → 302 + Location(code, state)
    let resp = authorize!(&app, ast, "code", CLIENT_ID, RP_REDIRECT, "st9", "n9");
    assert_eq!(resp.status(), 302, "authorize 应 302");
    let location = resp
        .headers()
        .get(LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("Location header")
        .to_string();
    assert!(
        location.starts_with(&format!("{RP_REDIRECT}?code=")),
        "location={location}"
    );
    assert!(location.contains("state=st9"));
    let code = location
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();

    // token 兑换 → 200 + id_token（public client：空 secret 放行）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/oidc/token")
            .insert_header((CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .set_payload(format!(
                "grant_type=authorization_code&code={code}&redirect_uri={RP_REDIRECT}\
                 &client_id={CLIENT_ID}&client_secret="
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "token 兑换应 200");
    let token_resp: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(token_resp["token_type"], "Bearer");
    let id_token = token_resp["id_token"]
        .as_str()
        .expect("id_token")
        .to_string();

    // JWKS 公钥验签 id_token（iss/sub/aud/nonce + exp）
    let jwks_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/.well-known/jwks.json")
            .to_request(),
    )
    .await;
    let jwks: serde_json::Value = test::read_body_json(jwks_resp).await;
    let jwk = &jwks["keys"][0];
    let key = jsonwebtoken::DecodingKey::from_ec_components(
        jwk["x"].as_str().unwrap(),
        jwk["y"].as_str().unwrap(),
    )
    .unwrap();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
    validation.validate_exp = true;
    validation.set_audience(&[CLIENT_ID]);
    let decoded = jsonwebtoken::decode::<jwt::OidcIdTokenClaims>(&id_token, &key, &validation)
        .expect("id_token 应可经 JWKS 验签");
    assert_eq!(decoded.claims.iss, ISSUER);
    assert_eq!(decoded.claims.sub, "424242");
    assert_eq!(decoded.claims.aud, CLIENT_ID);
    assert_eq!(decoded.claims.nonce.as_deref(), Some("n9"));

    // code 单次使用：重放 → 400 INVALID_GRANT
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/oidc/token")
            .insert_header((CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .set_payload(format!(
                "grant_type=authorization_code&code={code}&redirect_uri={RP_REDIRECT}\
                 &client_id={CLIENT_ID}&client_secret="
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 400, "code 重放应 400");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["code"], "INVALID_GRANT");
}

#[actix_web::test]
async fn oidc_authorize_negative_paths() {
    let (pool, ast) = build_deps().await;
    let app = init_app!(pool, ast);

    // response_type != code → 400
    let resp = authorize!(&app, ast, "token", CLIENT_ID, RP_REDIRECT, "s", "n");
    assert_eq!(resp.status(), 400, "response_type=token 应 400");

    // redirect_uri 越白名单 → 400
    let resp = authorize!(
        &app,
        ast,
        "code",
        CLIENT_ID,
        "https://evil.example.com/cb",
        "s",
        "n"
    );
    assert_eq!(resp.status(), 400, "越界 redirect_uri 应 400");
}

#[actix_web::test]
async fn oidc_token_wrong_client_secret_rejected() {
    let (pool, ast) = build_deps().await;
    sqlx::query(
        "INSERT INTO isahl_auth.oidc_clients (client_id, client_secret_hash, enabled) \
         VALUES ('secret-app', $1, TRUE) ON CONFLICT (client_id) DO UPDATE \
         SET client_secret_hash = $1, deleted_at = NULL",
    )
    .bind(hash_client_secret("right-secret").unwrap())
    .execute(&pool)
    .await
    .expect("seed secret client");

    let app = init_app!(pool, ast);
    let resp = authorize!(&app, ast, "code", "secret-app", RP_REDIRECT, "s", "n");
    assert_eq!(resp.status(), 302);
    let location = resp
        .headers()
        .get(LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let code = location
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();

    // 错误 secret → 401 INVALID_CLIENT
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/oidc/token")
            .insert_header((CONTENT_TYPE, "application/x-www-form-urlencoded"))
            .set_payload(format!(
                "grant_type=authorization_code&code={code}&redirect_uri={RP_REDIRECT}\
                 &client_id=secret-app&client_secret=wrong"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 401, "错误 client secret 应 401");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["code"], "INVALID_CLIENT");

    // 清理
    sqlx::query("DELETE FROM isahl_auth.oidc_clients WHERE client_id = 'secret-app'")
        .execute(&pool)
        .await
        .ok();
}
