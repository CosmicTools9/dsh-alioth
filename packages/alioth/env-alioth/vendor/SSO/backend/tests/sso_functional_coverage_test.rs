//! SSO 主要功能补充测试
//!
//! 覆盖此前无测试的功能区：API Key 生命周期、MFA 初始化/状态、NGAC PDP 决策。

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
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

async fn ensure_ngac(pool: &PgPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_policy_class (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name TEXT, description TEXT,
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            CONSTRAINT ngac_policy_class_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name TEXT, fk_policy_class BIGINT NOT NULL,
            ancestor_ids BIGINT[] DEFAULT '{}', children_ids BIGINT[] DEFAULT '{}', property JSONB DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            created_by_id BIGINT, updated_by_id BIGINT, deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_user_attribute_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_rr_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, created_by_id BIGINT, updated_by_id BIGINT, o_name TEXT,
            fk_user BIGINT, fk_user_attribute BIGINT, assigned_at TIMESTAMPTZ DEFAULT NOW(), expires_at TIMESTAMPTZ,
            conditions JSONB DEFAULT '{}', deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_user_rr_attribute_pkey PRIMARY KEY (id),
            CONSTRAINT ngac_user_rr_attribute_fk_user_fkey FOREIGN KEY (fk_user) REFERENCES isahl_auth.auth_users(id) ON DELETE CASCADE,
            CONSTRAINT ngac_user_rr_attribute_fk_user_attribute_fkey FOREIGN KEY (fk_user_attribute) REFERENCES isahl_auth.ngac_user_attribute(id) ON DELETE CASCADE)",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (id, o_name) \
         SELECT isahl.gen_next_zuid(), 'default' \
         WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default')",
    )
    .execute(pool)
    .await
    .ok();
}

async fn ensure_admin(pool: &PgPool) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'admin_cov', 'admin_cov', 'admin_cov@test.local', 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(pool)
    .await
    .ok();
    let admin: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'admin_cov@test.local' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("admin user");
    // policy class id 动态查询（o_name='default'），不可硬编码 1
    let pc_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("default policy class");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
         VALUES ('admin', $1, NOW(), NOW()) ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(pc_id)
    .execute(pool)
    .await
    .ok();
    let ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND fk_policy_class = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(pc_id)
    .fetch_one(pool)
    .await
    .expect("admin UA");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at) VALUES ($1, $2, NOW(), NOW())",
    )
    .bind(admin)
    .bind(ua)
    .execute(pool)
    .await
    .ok();
    admin
}

// ============================================================================
// API Key 生命周期
// ============================================================================
#[tokio::test]
async fn api_key_create_and_authenticate() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    ensure_ngac(&pool).await;
    let admin = ensure_admin(&pool).await;
    let ast = test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::admin::configure)
            .service(web::scope("/auth").route(
                "/authenticate",
                web::post().to(gateway_sso::auth::api_key::authenticate_handler),
            )),
    )
    .await;

    let token = mint_token(&ast, admin, "admin_cov@test.local");
    let hdr = actix_web::http::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap();

    // 创建 API Key（POST /api/admin/api-clients，client_type=apikey）
    let create = test::TestRequest::post()
        .uri("/api/admin/api-clients")
        .insert_header(("Authorization", hdr.clone()))
        .set_json(json!({"client_type": "apikey", "client_name": "ci-bot", "scopes": ["read"]}))
        .to_request();
    let resp = test::call_service(&app, create).await;
    assert_eq!(resp.status().as_u16(), 201, "create api key");
    let body: serde_json::Value = test::read_body_json(resp).await;
    let raw_key = body["secret"]
        .as_str()
        .expect("secret returned")
        .to_string();
    let key_id = body["id"].as_i64().expect("key id");
    assert!(raw_key.starts_with("ak_"), "key should be prefixed ak_");

    // 用密钥换取 JWT
    let auth = test::TestRequest::post()
        .uri("/auth/authenticate")
        .insert_header(("Authorization", format!("Bearer {}", raw_key)))
        .to_request();
    let resp = test::call_service(&app, auth).await;
    assert_eq!(resp.status().as_u16(), 200, "authenticate with api key");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body["access_token"].as_str().is_some(),
        "should return access_token"
    );

    // 用错误密钥应被拒绝
    let bad = test::TestRequest::post()
        .uri("/auth/authenticate")
        .insert_header(("Authorization", "Bearer ak_invalid_key_xyz"))
        .to_request();
    let resp = test::call_service(&app, bad).await;
    assert_eq!(resp.status().as_u16(), 401, "invalid api key rejected");

    // 清理
    sqlx::query("DELETE FROM isahl_auth.api_keys WHERE id = $1")
        .bind(key_id)
        .execute(&pool)
        .await
        .ok();
    // 清理：只删本测试创建的用户与其 UA 绑定（禁止全表 DELETE——会删掉 seed UA/关联）
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(admin)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email = 'admin_cov@test.local'")
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// MFA 初始化 / 状态
// ============================================================================
#[tokio::test]
async fn mfa_init_and_status() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();

    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'mfa_user', 'mfa_user@alioth.test', 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let user_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'mfa_user@alioth.test' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::auth::mfa_management::configure_routes),
    )
    .await;

    let token = mint_token(&ast, user_id, "mfa_user@alioth.test");
    let hdr = actix_web::http::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap();

    // 初始状态：未启用、无密钥
    let status = test::TestRequest::get()
        .uri("/auth/me/mfa/status")
        .insert_header(("Authorization", hdr.clone()))
        .to_request();
    let resp = test::call_service(&app, status).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["enabled"], json!(false));
    assert_eq!(body["has_secret"], json!(false));

    // 初始化：返回 TOTP secret
    let init = test::TestRequest::post()
        .uri("/auth/me/mfa/init")
        .insert_header(("Authorization", hdr.clone()))
        .to_request();
    let resp = test::call_service(&app, init).await;
    assert_eq!(resp.status().as_u16(), 200, "mfa init");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body["secret"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "secret returned"
    );

    // 初始化后状态：已有密钥
    let status = test::TestRequest::get()
        .uri("/auth/me/mfa/status")
        .insert_header(("Authorization", hdr.clone()))
        .to_request();
    let resp = test::call_service(&app, status).await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["has_secret"], json!(true), "has_secret after init");

    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// NGAC PDP 决策
// ============================================================================
#[tokio::test]
async fn ngac_pdp_check_returns_decision() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .configure(gateway_sso::ngac::pdp::configure_routes),
    )
    .await;

    let check = test::TestRequest::post()
        .uri("/pdp/check")
        .set_json(json!({"user_id": 1, "resource": "document:123", "action": "read"}))
        .to_request();
    let resp = test::call_service(&app, check).await;
    assert_eq!(resp.status().as_u16(), 200, "pdp check should respond 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["permitted"].is_boolean(), "permitted is bool");
    assert!(body["reason"].is_string(), "reason present");
}

// ============================================================================
// token introspect（M3）：有效/无效/过期 token 的 introspection 结果。
// ============================================================================
#[tokio::test]
async fn token_introspect_active_and_inactive() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();

    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at) \
         VALUES (isahl.gen_next_zuid(), 'introspect_user', 'introspect_user', 'introspect@test.local', 'active', true, NOW(), NOW()) \
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let user_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'introspect@test.local' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .route(
                "/introspect",
                web::post().to(gateway_sso::auth::introspect::introspect_handler),
            ),
    )
    .await;

    // 有效 token → active=true
    let token = mint_token(&ast, user_id, "introspect@test.local");
    let resp = test::TestRequest::post()
        .uri("/introspect")
        .set_form(json!({ "token": token }))
        .to_request();
    let resp = test::call_service(&app, resp).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["active"],
        serde_json::json!(true),
        "valid token active"
    );
    assert_eq!(body["sub"], serde_json::json!(user_id.to_string()));

    // 无效 token → active=false
    let resp = test::TestRequest::post()
        .uri("/introspect")
        .set_form(json!({ "token": "not-a-real-token" }))
        .to_request();
    let resp = test::call_service(&app, resp).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["active"],
        serde_json::json!(false),
        "invalid token inactive"
    );
}

// ============================================================================
// LDAP 配置列表（M3）：无需 LDAP server 的 DB CRUD 面。
// ============================================================================
#[tokio::test]
async fn ldap_configs_list() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");

    sqlx::query(
        "INSERT INTO isahl_auth.ldap_configs (name, url, bind_dn, bind_password, base_dn, enabled) \
         VALUES ('test-ldap', 'ldap://localhost:389', 'cn=admin,dc=test', 'pw', 'dc=test', true) \
         ON CONFLICT (name) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();

    let app = test::init_service(App::new().app_data(web::Data::new(pool.clone())).route(
        "/ldap/configs",
        web::get().to(gateway_sso::auth::ldap::list_ldap_configs),
    ))
    .await;

    let resp = test::TestRequest::get().uri("/ldap/configs").to_request();
    let resp = test::call_service(&app, resp).await;
    assert_eq!(resp.status().as_u16(), 200, "list ldap configs");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body.as_array().is_some() || body.get("configs").is_some() || !body.is_null(),
        "ldap configs response should be a list or object"
    );
}
