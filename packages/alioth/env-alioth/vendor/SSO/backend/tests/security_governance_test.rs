//! M23 安全收口集成测试（P0）：
//!   - NGAC PIP 写端点鉴权：旧 noauth 路径 404；新管理面路径无凭证 401、admin 201、
//!     regular 403
//!   - zchat 令牌治理：password grant 绑定会话（sid 非空）且 iss/aud 正确；
//!     refresh grant 拒绝过期/吊销 token 与被撤销会话
//!
//! 核心原则：测试数据绝不残留（与 tests/common 约定一致）。

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::{
    jwt::{self, encode_access_token, Claims},
    password::hash_password,
    session::{CreateSessionRequest, SessionManager},
    zchat, AuthState,
};
use serde_json::json;
use sqlx::PgPool;

const TEST_ISSUER: &str = "http://localhost:9002"; // jwt::token_config() 默认绑定值

async fn setup_pool() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

/// 幂等创建 refresh_tokens 表（测试 schema 不含该表 DDL）。
async fn ensure_refresh_tokens_table(pool: &PgPool) {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.refresh_tokens (
            id bigint DEFAULT isahl.gen_next_zuid() NOT NULL,
            user_id bigint NOT NULL,
            token_hash character varying(64) NOT NULL,
            expires_at timestamp with time zone NOT NULL,
            revoked boolean DEFAULT false,
            created_at timestamp with time zone DEFAULT now() NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create refresh_tokens table");
}

/// 幂等创建 NGAC 管理面（sso_admin OA + admin UA + 关联），与迁移 019/020 同构。
/// 保证 decide_access 有策略可判定（避免 bootstrap 兜底放行）。
async fn ensure_sso_admin_policy(pool: &PgPool) -> i64 {
    let pc_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("default policy class must exist");

    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
        VALUES ('admin', $1, NOW(), NOW())
        ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING
        "#,
    )
    .bind(pc_id)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_object_attribute
            (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_at)
        SELECT 'sso-admin', $1, 'sso_admin', 0, 'sso-admin', '{}'::jsonb, NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = 'sso_admin' AND fk_resource = 0 AND deleted_at IS NULL
        )
        "#,
    )
    .bind(pc_id)
    .execute(pool)
    .await
    .ok();
    let sso_admin_oa_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = 'sso_admin' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("sso_admin OA must exist");

    let admin_ua_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute \
         WHERE o_name = 'admin' AND fk_policy_class = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(pc_id)
    .fetch_one(pool)
    .await
    .expect("admin UA must exist");

    for ar in ["read", "write", "delete", "admin", "create"] {
        sqlx::query("INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) ON CONFLICT (o_name) DO NOTHING")
            .bind(ar)
            .execute(pool)
            .await
            .ok();
    }
    let ar_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right \
         WHERE o_name = ANY($1) ORDER BY o_name",
    )
    .bind(["read", "write", "delete", "admin", "create"])
    .fetch_all(pool)
    .await
    .expect("access rights must exist");

    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
        SELECT $1, $2, $3, $4, NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_association
            WHERE fk_user_attribute = $1 AND fk_object_attribute = $2 AND deleted_at IS NULL
        )
        "#,
    )
    .bind(admin_ua_id)
    .bind(sso_admin_oa_id)
    .bind(&ar_ids)
    .bind(pc_id)
    .execute(pool)
    .await
    .ok();

    sso_admin_oa_id
}

async fn create_user(pool: &PgPool, email: &str, password: &str) -> i64 {
    let password_hash = hash_password(password).expect("hash_password should succeed");
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.auth_users (name, username, email, password_hash, status, is_active, created_at, updated_at)
        VALUES ($1, $1, $2, $3, 'active', true, NOW(), NOW())
        ON CONFLICT (email) DO NOTHING
        "#,
    )
    .bind(email)
    .bind(email)
    .bind(password_hash)
    .execute(pool)
    .await
    .ok();
    sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("user should exist")
}

async fn assign_admin_ua(pool: &PgPool, user_id: i64) {
    let admin_ua_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute \
         WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("admin UA must exist");
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
        VALUES ($1, $2, NOW(), NOW())
        ON CONFLICT (fk_user, fk_user_attribute) DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind(admin_ua_id)
    .execute(pool)
    .await
    .expect("assign admin UA should succeed");
}

fn admin_token(state: &AuthState, user_id: i64) -> String {
    let claims = Claims::with_expiry_seconds(&user_id.to_string(), "", true, 900);
    encode_access_token(&claims, &state.jwt_private_key).expect("encode admin token")
}

async fn cleanup_user(pool: &PgPool, email: &str) {
    sqlx::query(
        "DELETE FROM isahl_auth.sso_sessions \
         WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)",
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.refresh_tokens \
         WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)",
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)",
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email = $1")
        .bind(email)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// PIP 端点鉴权
// ============================================================================

#[tokio::test]
async fn test_pip_legacy_noauth_path_returns_404() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let app = test::init_service(
        App::new()
            .wrap(gateway_sso::auth::middleware::RequireAuth::new())
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_auth_state()))
            .configure(gateway_sso::configure_protected_routes),
    )
    .await;

    // 旧 noauth 路径：路由已移除 → 404（且 /api/ngac 仍在公开匹配器，无需 JWT）。
    let req = test::TestRequest::post()
        .uri("/api/ngac/pip/users/1/attributes")
        .set_json(json!({"fk_user_attribute": 1}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        404,
        "legacy /api/ngac/pip path must be gone"
    );
}

#[tokio::test]
async fn test_pip_new_admin_path_auth_chain() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("setup schema");
    ensure_sso_admin_policy(&pool).await;

    let admin_email = "pip-admin@test.local";
    let regular_email = "pip-regular@test.local";
    let admin_id = create_user(&pool, admin_email, "AdminPass123!").await;
    let regular_id = create_user(&pool, regular_email, "RegularPass123!").await;
    assign_admin_ua(&pool, admin_id).await;

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .wrap(gateway_sso::auth::middleware::RequireAuth::new())
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .configure(gateway_sso::configure_protected_routes),
    )
    .await;

    // 目标 UA（admin UA 自身作为被指派对象，验证写端点落库）
    let ua_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("admin UA");

    let uri = format!("/api/admin/ngac/pip/users/{}/attributes", regular_id);

    // 1) 无凭证 → 401（RequireAuth 拦截 /api/admin/*）
    let req = test::TestRequest::post()
        .uri(&uri)
        .set_json(json!({"fk_user_attribute": ua_id}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401, "no credentials must be 401");

    // 2) 非 admin JWT → 403（NgacPep sso_admin OA 决策拒绝）
    let regular_token = admin_token(&auth_state, regular_id);
    let req = test::TestRequest::post()
        .uri(&uri)
        .insert_header(("Authorization", format!("Bearer {}", regular_token)))
        .set_json(json!({"fk_user_attribute": ua_id}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 403, "non-admin user must be denied");

    // 3) admin JWT → 201 且属性指派落库
    let admin_token = admin_token(&auth_state, admin_id);
    let req = test::TestRequest::post()
        .uri(&uri)
        .insert_header(("Authorization", format!("Bearer {}", admin_token)))
        .set_json(json!({"fk_user_attribute": ua_id}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        201,
        "admin must be allowed to assign"
    );

    let assigned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL)",
    )
    .bind(regular_id)
    .bind(ua_id)
    .fetch_one(&pool)
    .await
    .expect("assignment check");
    assert!(assigned, "attribute assignment must be persisted");

    // 4) admin GET 同路径 → 200
    let req = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("Authorization", format!("Bearer {}", admin_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "admin GET must be allowed");

    cleanup_user(&pool, admin_email).await;
    cleanup_user(&pool, regular_email).await;
}

// ============================================================================
// zchat 令牌治理
// ============================================================================

#[tokio::test]
async fn test_zchat_password_grant_binds_session_and_iss_aud() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let email = "zchat-pwd@test.local";
    let user_id = create_user(&pool, email, "ZchatPass123!").await;

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(zchat::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/auth/zchat")
        .set_json(json!({
            "grant_type": "password",
            "username": email,
            "password": "ZchatPass123!"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "password grant should succeed");
    let body: serde_json::Value = test::read_body_json(resp).await;

    let claims = jwt::decode_token(
        body["access_token"].as_str().expect("access_token"),
        &auth_state.jwt_public_key,
    )
    .expect("issued token must be decodable");
    assert!(
        !claims.sid.is_empty(),
        "sid must be non-empty (session-bound)"
    );
    assert_eq!(
        claims.iss, TEST_ISSUER,
        "iss must match token validation config"
    );
    assert_eq!(
        claims.aud, TEST_ISSUER,
        "aud must match token validation config"
    );
    assert_eq!(claims.protocol, "zchat");

    // 会话确实落库
    let session_alive: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM isahl_auth.sso_sessions \
         WHERE session_token = $1 AND status = 'active')",
    )
    .bind(&claims.sid)
    .fetch_one(&pool)
    .await
    .expect("session check");
    assert!(session_alive, "session must be persisted as active");

    let _ = user_id;
    cleanup_user(&pool, email).await;
}

#[tokio::test]
async fn test_zchat_refresh_grant_validation() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("setup schema");
    ensure_refresh_tokens_table(&pool).await;

    let email = "zchat-refresh@test.local";
    let user_id = create_user(&pool, email, "ZchatPass123!").await;
    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(zchat::configure)),
    )
    .await;

    // 构造有效会话 + refresh token（与 login 流程同构：encode_refresh_token + 落库哈希）
    let session_manager = SessionManager::new(pool.clone());
    let session = session_manager
        .create_session(CreateSessionRequest {
            user_id,
            ..Default::default()
        })
        .await
        .expect("create session");

    let mut claims = Claims::with_expiry_seconds(&user_id.to_string(), "", false, 900);
    claims.sid = session.session_token.clone();
    let valid_refresh = jwt::encode_refresh_token(&claims, &auth_state.jwt_private_key, 604800)
        .expect("encode refresh token");
    sqlx::query(
        "INSERT INTO isahl_auth.refresh_tokens (user_id, token_hash, expires_at) \
         VALUES ($1, $2, NOW() + INTERVAL '7 days')",
    )
    .bind(user_id)
    .bind(sha256_hex(&valid_refresh))
    .execute(&pool)
    .await
    .expect("store refresh token hash");

    // 1) 有效 refresh + 活动会话 → 200
    let req = test::TestRequest::post()
        .uri("/auth/auth/zchat")
        .set_json(json!({"grant_type": "refresh_token", "refresh_token": valid_refresh}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "valid refresh must succeed");

    // 2) 过期 refresh（exp 已过）→ 401
    let expired = jwt::encode_refresh_token(&claims, &auth_state.jwt_private_key, -3600)
        .expect("encode expired refresh token");
    let req = test::TestRequest::post()
        .uri("/auth/auth/zchat")
        .set_json(json!({"grant_type": "refresh_token", "refresh_token": expired}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401, "expired refresh must be 401");

    // 3) refresh token 被 revoke → 401
    sqlx::query("UPDATE isahl_auth.refresh_tokens SET revoked = TRUE WHERE token_hash = $1")
        .bind(sha256_hex(&valid_refresh))
        .execute(&pool)
        .await
        .expect("revoke refresh token");
    let req = test::TestRequest::post()
        .uri("/auth/auth/zchat")
        .set_json(json!({"grant_type": "refresh_token", "refresh_token": valid_refresh}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401, "revoked refresh must be 401");

    // 4) 会话被 revoke（管理员路径：只改 sso_sessions.status）→ 401
    //    重新签发未 revoked 的 refresh token
    let valid_refresh2 = jwt::encode_refresh_token(&claims, &auth_state.jwt_private_key, 604800)
        .expect("encode refresh token 2");
    sqlx::query(
        "INSERT INTO isahl_auth.refresh_tokens (user_id, token_hash, expires_at) \
         VALUES ($1, $2, NOW() + INTERVAL '7 days')",
    )
    .bind(user_id)
    .bind(sha256_hex(&valid_refresh2))
    .execute(&pool)
    .await
    .expect("store refresh token hash 2");
    sqlx::query("UPDATE isahl_auth.sso_sessions SET status = 'revoked' WHERE session_token = $1")
        .bind(&session.session_token)
        .execute(&pool)
        .await
        .expect("revoke session");
    let req = test::TestRequest::post()
        .uri("/auth/auth/zchat")
        .set_json(json!({"grant_type": "refresh_token", "refresh_token": valid_refresh2}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401, "revoked session must be 401");

    cleanup_user(&pool, email).await;
}
