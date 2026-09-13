//! fix-sso-auth-gaps 缺口修复集成测试
//!
//! 覆盖：
//! - G4 housekeeping 周期清理（超保留期会话/刷新令牌物理删除 + 子表级联 + 有效行保留）
//! - G6c 签发订阅门禁（/auth/token client_credentials 无/有激活订阅）
//! - P1 注册邮箱验证负路径（未验证 email → 400 EMAIL_NOT_VERIFIED）
//! - P2 SCIM Groups PATCH（add/remove/replace/非法请求 400）
//!
//! 注：运行需共享测试库（aliothstudio_test）已含 isahl_auth 基础表与 isahl schema
//! 审批链（与既有 sso-gap 系列测试同假设）。测试数据用完即清，不残留。

mod common;

use actix_web::{test, web, App};
use serde_json::json;
use sqlx::PgPool;

async fn setup_pool() -> PgPool {
    ::common::testing::connect_test_db().await
}

async fn create_test_user(pool: &PgPool, tag: &str) -> i64 {
    let email = format!("gaps_{}_{}@test.local", tag, uuid::Uuid::new_v4().simple());
    sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users (name, username, email, password_hash, status, user_type, is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, 'hash', 'active', 'standard', true, NOW(), NOW()) RETURNING id",
    )
    .bind(&email)
    .bind(format!("{}_{}", tag, uuid::Uuid::new_v4().simple()))
    .bind(&email)
    .fetch_one(pool)
    .await
    .expect("create test user")
}

async fn delete_user(pool: &PgPool, user_id: i64) {
    sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.refresh_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_user_emails WHERE fk_user = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .ok();
}

// ── G4：housekeeping 周期清理 ─────────────────────────────────────────────────

#[tokio::test]
async fn housekeeping_purges_expired_sessions_and_tokens() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();

    let user_id = create_test_user(&pool, "hk").await;

    // 超保留期（40 天前过期）expired 会话 + 关联 revocation 行
    let old_session: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.sso_sessions (user_id, session_token, status, created_at, last_activity_at, expires_at) \
         VALUES ($1, $2, 'expired', NOW() - INTERVAL '45 days', NOW() - INTERVAL '45 days', NOW() - INTERVAL '40 days') RETURNING id",
    )
    .bind(user_id)
    .bind(format!("old-expired-{}", uuid::Uuid::new_v4().simple()))
    .fetch_one(&pool)
    .await
    .expect("insert old expired session");
    sqlx::query(
        "INSERT INTO isahl_auth.session_revocations (id, session_id, user_id, revoked_at) \
         VALUES (isahl.gen_next_zuid(), $1, $2, NOW() - INTERVAL '40 days')",
    )
    .bind(old_session)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("insert revocation child row");

    // 超保留期 revoked 会话
    let old_revoked: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.sso_sessions (user_id, session_token, status, created_at, last_activity_at, expires_at) \
         VALUES ($1, $2, 'revoked', NOW() - INTERVAL '45 days', NOW() - INTERVAL '45 days', NOW() - INTERVAL '40 days') RETURNING id",
    )
    .bind(user_id)
    .bind(format!("old-revoked-{}", uuid::Uuid::new_v4().simple()))
    .fetch_one(&pool)
    .await
    .expect("insert old revoked session");

    // 保留：active 未过期会话
    let active_session: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.sso_sessions (user_id, session_token, status, expires_at) \
         VALUES ($1, $2, 'active', NOW() + INTERVAL '1 hour') RETURNING id",
    )
    .bind(user_id)
    .bind(format!("active-{}", uuid::Uuid::new_v4().simple()))
    .fetch_one(&pool)
    .await
    .expect("insert active session");

    // 超保留期 revoked / 过期 refresh token + 保留的活跃 token
    let old_token = format!("old-rt-{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO isahl_auth.refresh_tokens (user_id, token_hash, expires_at, revoked, created_at) \
         VALUES ($1, $2, NOW() - INTERVAL '40 days', TRUE, NOW() - INTERVAL '45 days')",
    )
    .bind(user_id)
    .bind(&old_token)
    .execute(&pool)
    .await
    .expect("insert old revoked token");
    let live_token = format!("live-rt-{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO isahl_auth.refresh_tokens (user_id, token_hash, expires_at, revoked, created_at) \
         VALUES ($1, $2, NOW() + INTERVAL '7 days', FALSE, NOW())",
    )
    .bind(user_id)
    .bind(&live_token)
    .execute(&pool)
    .await
    .expect("insert live token");

    gateway_sso::housekeeping::run_once(&pool)
        .await
        .expect("housekeeping round");

    let old_session_gone: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM isahl_auth.sso_sessions WHERE id = $1)",
    )
    .bind(old_session)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(old_session_gone, "expired>retention session must be purged");
    let revocation_gone: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM isahl_auth.session_revocations WHERE session_id = $1)",
    )
    .bind(old_session)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(revocation_gone, "child revocation row must cascade-delete");
    let old_revoked_gone: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM isahl_auth.sso_sessions WHERE id = $1)",
    )
    .bind(old_revoked)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(old_revoked_gone, "revoked>retention session must be purged");
    let active_kept: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM isahl_auth.sso_sessions WHERE id = $1 AND status = 'active')",
    )
    .bind(active_session)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(active_kept, "active session must be kept");
    let old_token_gone: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM isahl_auth.refresh_tokens WHERE token_hash = $1)",
    )
    .bind(old_token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(old_token_gone, "revoked>retention token must be purged");
    let live_token_kept: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM isahl_auth.refresh_tokens WHERE token_hash = $1)",
    )
    .bind(live_token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(live_token_kept, "live token must be kept");

    // 清理：先删会话行与 token，再删用户（FK 顺序）
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
    delete_user(&pool, user_id).await;
}

// ── G6c：/auth/token 签发订阅门禁 ─────────────────────────────────────────────

#[tokio::test]
async fn client_credentials_requires_active_subscription() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .route(
                "/auth/token",
                web::post().to(gateway_sso::auth::token::token_handler),
            ),
    )
    .await;

    // client 数据准备（无订阅）
    let user_id = create_test_user(&pool, "sub").await;
    let client_uuid = format!("gaps-{}", uuid::Uuid::new_v4().simple());
    let secret = uuid::Uuid::new_v4().to_string();
    let secret_hash = gateway_sso::auth::client_secret::hash_client_secret_async(secret.clone())
        .await
        .expect("hash secret");
    let client_id_i64: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.api_clients (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled) \
         VALUES ($1, 'oauth2', 'gaps-test', $2, ARRAY['read'], $3, true) RETURNING id",
    )
    .bind(&client_uuid)
    .bind(&secret_hash)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("insert client");

    let token_req = |client_id: &str, client_secret: &str| {
        test::TestRequest::post()
            .uri("/auth/token")
            .set_form(json!({
                "grant_type": "client_credentials",
                "client_id": client_id,
                "client_secret": client_secret,
                "scope": "read",
            }))
            .to_request()
    };

    // 1. 无订阅 → 401 SUBSCRIPTION_INACTIVE
    let resp = test::call_service(&app, token_req(&client_uuid, &secret)).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "no-subscription client must be rejected"
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body["error_description"]
            .as_str()
            .unwrap_or("")
            .contains("SUBSCRIPTION_INACTIVE"),
        "expected SUBSCRIPTION_INACTIVE, got {:?}",
        body
    );

    // 2. 补激活订阅 → 200 签发
    let plan_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.api_plans (code, tier, rate_limit_rps, burst, quota_daily, quota_monthly, enabled) \
         VALUES ('gaps-test-plan', 0, 1.0, 5, 1000, 30000, true) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert plan");
    sqlx::query(
        "INSERT INTO isahl_auth.api_subscriptions (fk_client, fk_plan, status, starts_at) VALUES ($1, $2, 'active', NOW())",
    )
    .bind(client_id_i64)
    .bind(plan_id)
    .execute(&pool)
    .await
    .expect("insert subscription");

    let resp = test::call_service(&app, token_req(&client_uuid, &secret)).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "active-subscription client must be issued"
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["access_token"].as_str().is_some());

    // 3. 订阅过期 → 再次 401
    sqlx::query("UPDATE isahl_auth.api_subscriptions SET expires_at = NOW() - INTERVAL '1 day' WHERE fk_client = $1")
        .bind(client_id_i64)
        .execute(&pool)
        .await
        .expect("expire subscription");
    let resp = test::call_service(&app, token_req(&client_uuid, &secret)).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expired subscription must be rejected"
    );

    // 清理
    sqlx::query("DELETE FROM isahl_auth.api_subscriptions WHERE fk_client = $1")
        .bind(client_id_i64)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.api_plans WHERE id = $1")
        .bind(plan_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.api_clients WHERE id = $1")
        .bind(client_id_i64)
        .execute(&pool)
        .await
        .ok();
    delete_user(&pool, user_id).await;
}

// ── P1：注册邮箱验证负路径 ─────────────────────────────────────────────────────

#[tokio::test]
async fn register_rejects_unverified_email() {
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

    let email = format!("unverified_{}@test.local", uuid::Uuid::new_v4().simple());
    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": email,
            "username": format!("uv_{}", uuid::Uuid::new_v4().simple()),
            "password": "TestPass123!"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "unverified email registration must be rejected"
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("EMAIL_NOT_VERIFIED"),
        "expected EMAIL_NOT_VERIFIED, got {:?}",
        body
    );
    // 无用户行残留
    let leftover: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM isahl_auth.auth_users WHERE email = $1")
            .bind(&email)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(leftover, 0, "no user row may be created");

    // 清理验证码残留（如成功路径并发写入）
    sqlx::query("DELETE FROM isahl_auth.auth_email_verifications WHERE email = $1")
        .bind(&email)
        .execute(&pool)
        .await
        .ok();
}

// ── P2：SCIM Groups PATCH ─────────────────────────────────────────────────────

async fn ensure_ngac_schema(pool: &PgPool) {
    // 幂等补 ngac 表（与 sso_functional_coverage_test 同源；共享测试库并发安全）
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
}

#[tokio::test]
async fn scim_groups_patch_lifecycle() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.ok();
    ensure_ngac_schema(&pool).await;

    std::env::set_var("SCIM_BEARER_TOKEN", "gaps-test-scim-token");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(web::scope("/scim/v2").configure(gateway_sso::scim::configure)),
    )
    .await;

    // 数据准备：default policy class + group + 2 users
    let pc_id: i64 = {
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        match existing {
            Some(id) => id,
            None => sqlx::query_scalar(
                "INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default') RETURNING id",
            )
            .fetch_one(&pool)
            .await
            .expect("insert default policy class"),
        }
    };

    let user_a = create_test_user(&pool, "scim_a").await;
    let user_b = create_test_user(&pool, "scim_b").await;
    let group_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("gaps-group-{}", uuid::Uuid::new_v4().simple()))
    .bind(pc_id)
    .fetch_one(&pool)
    .await
    .expect("insert group");

    let auth_header = ("Authorization", "Bearer gaps-test-scim-token");
    let patch_uri = format!("/scim/v2/Groups/{}", group_id);

    // 1. add 成员 A + B
    let req = test::TestRequest::patch()
        .uri(&patch_uri)
        .insert_header(auth_header)
        .set_json(json!({
            "Operations": [{
                "op": "add",
                "path": "members",
                "value": [{"value": user_a.to_string()}, {"value": user_b.to_string()}]
            }]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "add members should succeed");
    let members = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user_attribute = $1 AND deleted_at IS NULL",
    )
    .bind(group_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(members, 2, "both members added");

    // 2. remove 成员 A
    let req = test::TestRequest::patch()
        .uri(&patch_uri)
        .insert_header(auth_header)
        .set_json(json!({
            "Operations": [{
                "op": "remove",
                "path": "members",
                "value": [{"value": user_a.to_string()}]
            }]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "remove member should succeed");
    let a_kept: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user_attribute = $1 AND fk_user = $2 AND deleted_at IS NULL)",
    )
    .bind(group_id)
    .bind(user_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!a_kept, "member A must be removed");

    // 3. replace 全量（仅 B）→ 清 A 重加 B 语义已覆盖；再 replace 为空集验证清空
    let req = test::TestRequest::patch()
        .uri(&patch_uri)
        .insert_header(auth_header)
        .set_json(json!({
            "Operations": [{"op": "replace", "path": "members", "value": []}]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "replace members should succeed"
    );
    let members = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user_attribute = $1 AND deleted_at IS NULL",
    )
    .bind(group_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(members, 0, "replace with empty must clear members");

    // 4. displayName replace
    let req = test::TestRequest::patch()
        .uri(&patch_uri)
        .insert_header(auth_header)
        .set_json(json!({
            "Operations": [{"op": "replace", "path": "displayName", "value": "renamed-gaps-group"}]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "displayName replace should succeed"
    );
    let name: Option<String> =
        sqlx::query_scalar("SELECT o_name FROM isahl_auth.ngac_user_attribute WHERE id = $1")
            .bind(group_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(name.as_deref(), Some("renamed-gaps-group"));

    // 5. 未知 path → 400 且组数据不变
    let req = test::TestRequest::patch()
        .uri(&patch_uri)
        .insert_header(auth_header)
        .set_json(json!({
            "Operations": [{"op": "replace", "path": "nonexistent", "value": "x"}]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400, "unknown path must be rejected");

    // 6. 非法 member 引用 → 400（组数据不变）
    let req = test::TestRequest::patch()
        .uri(&patch_uri)
        .insert_header(auth_header)
        .set_json(json!({
            "Operations": [{"op": "add", "path": "members", "value": [{"value": "999999999999"}]}]
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "invalid member must be rejected"
    );
    let members = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user_attribute = $1 AND deleted_at IS NULL",
    )
    .bind(group_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(members, 0, "failed patch must not mutate group");

    // 7. 无 token → 401
    let req = test::TestRequest::patch()
        .uri(&patch_uri)
        .set_json(json!({"Operations": [{"op": "add", "path": "members", "value": []}]}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "missing bearer must be rejected"
    );

    // 清理
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user_attribute = $1")
        .bind(group_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE id = $1")
        .bind(group_id)
        .execute(&pool)
        .await
        .ok();
    delete_user(&pool, user_a).await;
    delete_user(&pool, user_b).await;
}
