//! SSO 功能缺口修复验证测试
//!
//! 覆盖四类已修复的缺口：
//!  1. notification_preferences 存储（迁移 016 + handler 读写真实列）
//!  2. SCIM 2.0 端点鉴权（从公开白名单移除 /scim/，middleware 现在要求 JWT）
//!  3. IdP 管理后台 CRUD（新增 get/delete/toggle/test 端点，与前端路径对齐）
//!  4. EPP EventHandler 接线（audit 写入路径经由 EPP 记录访问事件）

mod common;

use actix_web::{test, web, App};
use gateway_sso::audit::handlers::{handle_audit_event, AuditEventRecord, AuditEventType};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::middleware::RequireAuth;
use gateway_sso::auth::AuthState;
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

/// 为给定用户签发访问令牌（使用测试 ES256 密钥对）。
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

/// 创建 NGAC 基础表 + 默认 policy class（admin 检查前置条件）。
async fn ensure_ngac(pool: &PgPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_policy_class (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL,
            o_name TEXT, description TEXT,
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

/// 创建并绑定一个 admin 用户，返回 user_id。
async fn ensure_admin(pool: &PgPool) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'admin_gap', 'admin_gap', 'admin_gap@test.local', 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(pool)
    .await
    .ok();
    let admin: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'admin_gap@test.local' LIMIT 1",
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
// 1. notification_preferences 存储
// ============================================================================
#[tokio::test]
async fn notification_preferences_roundtrip() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();

    // 准备一个测试用户
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'np_user', 'np_user@alioth.test', 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let user_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'np_user@alioth.test' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::auth::notification_preferences::configure_routes),
    )
    .await;

    let token = mint_token(&ast, user_id, "np_user@alioth.test");
    let auth = format!("Bearer {}", token);

    // GET 默认偏好（不应 500）
    let get = test::TestRequest::get()
        .uri("/auth/me/notification-preferences")
        .insert_header(("Authorization", auth.clone()))
        .to_request();
    let resp = test::call_service(&app, get).await;
    assert_eq!(resp.status().as_u16(), 200, "GET prefs should succeed");

    // PUT 写入偏好
    let prefs = json!({
        "approval_enabled": true,
        "email_enabled": false,
        "quiet_hours_start": "22:00",
        "quiet_hours_end": "08:00"
    });
    let put = test::TestRequest::put()
        .uri("/auth/me/notification-preferences")
        .insert_header(("Authorization", auth.clone()))
        .set_json(&prefs)
        .to_request();
    let resp = test::call_service(&app, put).await;
    assert_eq!(resp.status().as_u16(), 200, "PUT prefs should succeed");

    // GET 应返回写入的值（验证持久化到真实列）
    let get = test::TestRequest::get()
        .uri("/auth/me/notification-preferences")
        .insert_header(("Authorization", auth))
        .to_request();
    let body: serde_json::Value = test::read_body_json(test::call_service(&app, get).await).await;
    assert_eq!(body["approval_enabled"], json!(true));
    assert_eq!(body["email_enabled"], json!(false));
    assert_eq!(body["quiet_hours_start"], json!("22:00"));

    // 直接校验 DB 列确实写入（修复前会因列不存在而失败）
    let stored: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT notification_preferences FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    let stored = stored.expect("notification_preferences should be stored");
    assert_eq!(stored["approval_enabled"], json!(true));

    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// 2. SCIM 鉴权
// ============================================================================
#[tokio::test]
async fn scim_requires_authentication() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();

    // 准备一个用户（用于合法令牌）
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'scim_user', 'scim_user@alioth.test', 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let user_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'scim_user@alioth.test' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .wrap(RequireAuth::new())
            .configure(gateway_sso::scim::configure),
    )
    .await;

    // 无令牌访问 SCIM → 必须 401（修复前 /scim/ 在公开白名单中会被放行）
    let anon = test::TestRequest::get().uri("/scim/v2/Users").to_request();
    let resp = test::call_service(&app, anon).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "SCIM must require auth (was public before fix)"
    );

    // 带合法令牌 → 通过 middleware（handler 自身可能 200/错误，但不应再是 401）
    let token = mint_token(&ast, user_id, "scim_user@alioth.test");
    let authed = test::TestRequest::get()
        .uri("/scim/v2/Users")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, authed).await;
    assert_ne!(
        resp.status().as_u16(),
        401,
        "SCIM with valid token should pass middleware"
    );

    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// 3. IdP 管理后台 CRUD
// ============================================================================
#[tokio::test]
async fn idp_admin_crud() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    ensure_ngac(&pool).await;
    let admin = ensure_admin(&pool).await;
    let ast = test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::admin::configure),
    )
    .await;

    let token = mint_token(&ast, admin, "admin_gap@test.local");
    let hdr = actix_web::http::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap();

    // LIST
    let list = test::TestRequest::get()
        .uri("/api/admin/providers")
        .insert_header(("Authorization", hdr.clone()))
        .to_request();
    let resp = test::call_service(&app, list).await;
    assert_eq!(resp.status().as_u16(), 200, "list providers");

    // CREATE
    let create = test::TestRequest::post()
        .uri("/api/admin/providers")
        .insert_header(("Authorization", hdr.clone()))
        .set_json(json!({
            "name": "test-oidc",
            "provider_type": "oidc",
            "enabled": true,
            "client_id": "cid",
            "client_secret": "secret",
            "config": {"authorization_endpoint": "https://x/a", "token_endpoint": "https://x/t", "client_id": "cid"}
        }))
        .to_request();
    let resp = test::call_service(&app, create).await;
    assert_eq!(resp.status().as_u16(), 201, "create provider");
    let body: serde_json::Value = test::read_body_json(resp).await;
    let pid = body["id"].as_i64().expect("provider id");

    // GET SINGLE（新增端点）
    let get = test::TestRequest::get()
        .uri(&format!("/api/admin/providers/{}", pid))
        .insert_header(("Authorization", hdr.clone()))
        .to_request();
    let resp = test::call_service(&app, get).await;
    assert_eq!(resp.status().as_u16(), 200, "get single provider");

    // UPDATE
    let upd = test::TestRequest::put()
        .uri(&format!("/api/admin/providers/{}", pid))
        .insert_header(("Authorization", hdr.clone()))
        .set_json(json!({"enabled": false}))
        .to_request();
    let resp = test::call_service(&app, upd).await;
    assert_eq!(resp.status().as_u16(), 200, "update provider");

    // TOGGLE（新增端点）
    let tog = test::TestRequest::post()
        .uri(&format!("/api/admin/providers/{}/toggle", pid))
        .insert_header(("Authorization", hdr.clone()))
        .to_request();
    let resp = test::call_service(&app, tog).await;
    assert_eq!(resp.status().as_u16(), 200, "toggle provider");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["enabled"],
        json!(true),
        "toggle flips enabled back to true"
    );

    // TEST（新增端点）
    let tst = test::TestRequest::post()
        .uri(&format!("/api/admin/providers/{}/test", pid))
        .insert_header(("Authorization", hdr.clone()))
        .to_request();
    let resp = test::call_service(&app, tst).await;
    assert_eq!(resp.status().as_u16(), 200, "test provider");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["valid"],
        json!(true),
        "provider config should be valid"
    );

    // DELETE（新增端点）
    let del = test::TestRequest::delete()
        .uri(&format!("/api/admin/providers/{}", pid))
        .insert_header(("Authorization", hdr.clone()))
        .to_request();
    let resp = test::call_service(&app, del).await;
    assert_eq!(resp.status().as_u16(), 204, "delete provider");

    // 清理
    sqlx::query("DELETE FROM isahl_auth.identity_providers WHERE id = $1")
        .bind(pid)
        .execute(&pool)
        .await
        .ok();
    // 清理：只删本测试创建的用户与其 UA 绑定。
    // 禁止全表 DELETE ngac_user_attribute / ngac_user_rr_attribute ——
    // 那会删除 005/019 seed 的 admin/operator 等 UA 与关联，破坏其他并行测试。
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(admin)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email = 'admin_gap@test.local'")
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// 4. EPP EventHandler 接线
// ============================================================================
#[tokio::test]
async fn epp_records_access_event() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");

    let record = AuditEventRecord {
        id: 0,
        event_type: AuditEventType::AccessAllowed,
        timestamp: chrono::Utc::now(),
        subject_id: 42,
        object_id: 7,
        object_type: "document/report".to_string(),
        operation: "read".to_string(),
        success: true,
        metadata: Some(json!({"source": "test"})),
    };

    let event_id = handle_audit_event(record, &pool)
        .await
        .expect("handle_audit_event should record via EPP");
    assert!(event_id > 0, "returned event id");

    // 验证访问事件确实写入 audit_events（EPP 之前从未被调用）
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_audit.audit_events WHERE id = $1 AND user_id = 42",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "EPP should have inserted an access event row");

    sqlx::query("DELETE FROM isahl_audit.audit_events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// 6. websocket 审计流（M2）：audit_decision（PDP 决策审计）→ 全局 AuditWsServer
//    广播 → 订阅者收到 audit_event。actix-web 4 无 test::ws，传输层由 actix 保证，
//    此处验证 producer→broadcast channel 完整链路。
// ============================================================================
#[tokio::test]
async fn ws_audit_stream_receives_broadcast() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .app_data(web::Data::new(gateway_sso::websocket::AppState::new()))
            .configure(gateway_sso::configure_protected_routes),
    )
    .await;

    // 订阅全局广播（与 ClientSession 同通道）
    let mut rx = gateway_sso::websocket::init_ws_server().client_receiver();

    // 触发一次 PDP 决策（经 audit_decision 广播到 WS）
    let decide = test::TestRequest::post()
        .uri("/api/ngac/decide")
        .set_json(json!({
            "user_id": 42,
            "resource": "sso_audit:0",
            "action": "read"
        }))
        .to_request();
    let _resp = test::call_service(&app, decide).await;

    // 广播通道应收审计事件（超时保护）
    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("broadcast should deliver within timeout")
        .expect("recv ok");
    assert_eq!(
        msg.msg_type,
        gateway_sso::websocket::WsMessageType::AuditEvent
    );
    assert_eq!(msg.payload["operation"], serde_json::json!("read"));
    assert_eq!(
        msg.payload["subject_id"],
        serde_json::json!("00000000-0000-0000-0000-00000000002a")
    );
}

// ============================================================================
// 5. 资源级 NGAC PEP（M1）：/api/admin 与 /api/audit 在 Enforce 模式下
//    （默认）必须按 sso_admin/sso_audit OA 关联决策——admin 放行、普通用户 403。
// ============================================================================
#[tokio::test]
async fn pep_enforces_admin_and_audit_resources() {
    let _ = env_logger::try_init();
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();

    // admin 用户：绑定 admin UA（005 seed 的 admin UA 已关联 sso_admin/sso_audit OA，019）
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at) \
         VALUES (isahl.gen_next_zuid(), 'pep_admin', 'pep_admin', 'pep_admin@test.local', 'active', true, NOW(), NOW()) \
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let admin_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'pep_admin@test.local' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("admin user");
    let admin_ua_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("admin UA");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at) \
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(admin_id)
    .bind(admin_ua_id)
    .execute(&pool)
    .await
    .ok();

    // 普通用户：仅默认 user UA（无 sso_admin/sso_audit 关联）
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at) \
         VALUES (isahl.gen_next_zuid(), 'pep_regular', 'pep_regular', 'pep_regular@test.local', 'active', true, NOW(), NOW()) \
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let regular_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'pep_regular@test.local' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("regular user");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::configure_protected_routes),
    )
    .await;

    let admin_auth = format!(
        "Bearer {}",
        mint_token(&ast, admin_id, "pep_admin@test.local")
    );
    let regular_auth = format!(
        "Bearer {}",
        mint_token(&ast, regular_id, "pep_regular@test.local")
    );

    // admin：/api/audit/events 与 /api/admin/users 均应放行（sso_audit/sso_admin OA 关联）
    let audit_admin = test::TestRequest::get()
        .uri("/api/audit/events")
        .insert_header(("Authorization", admin_auth.clone()))
        .to_request();
    let resp = test::call_service(&app, audit_admin).await;
    let audit_admin_status = resp.status().as_u16();
    if audit_admin_status != 200 {
        let body: serde_json::Value = test::read_body_json(resp).await;
        eprintln!("admin audit status={} body={}", audit_admin_status, body);
    }
    assert_eq!(audit_admin_status, 200, "admin can read audit events");

    let users_admin = test::TestRequest::get()
        .uri("/api/admin/users")
        .insert_header(("Authorization", admin_auth.clone()))
        .to_request();
    let resp = test::call_service(&app, users_admin).await;
    let users_admin_status = resp.status().as_u16();
    if users_admin_status != 200 {
        let body: serde_json::Value = test::read_body_json(resp).await;
        eprintln!("admin users status={} body={}", users_admin_status, body);
    }
    assert_eq!(users_admin_status, 200, "admin can list users");

    // 普通用户：均 403（无 sso_admin/sso_audit 关联 → NotApplicable → fail-closed）
    // 直接验证 PDP 决策（公开端点 /api/ngac/decide）：普通用户对 sso_audit:0 应为 deny
    let decide_resp = test::TestRequest::post()
        .uri("/api/ngac/decide")
        .set_json(json!({
            "user_id": regular_id,
            "resource": "sso_audit:0",
            "action": "read"
        }))
        .to_request();
    let resp = test::call_service(&app, decide_resp).await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["permitted"],
        serde_json::Value::Bool(false),
        "PDP must deny regular user on sso_audit:0"
    );

    let audit_regular = test::TestRequest::get()
        .uri("/api/audit/events")
        .insert_header(("Authorization", regular_auth.clone()))
        .to_request();
    let resp = test::call_service(&app, audit_regular).await;
    let regular_status = resp.status().as_u16();
    if regular_status != 403 {
        let body: serde_json::Value = test::read_body_json(resp).await;
        eprintln!("regular audit status={} body={}", regular_status, body);
    }
    assert_eq!(regular_status, 403, "regular user denied on audit events");

    let users_regular = test::TestRequest::get()
        .uri("/api/admin/users")
        .insert_header(("Authorization", regular_auth.clone()))
        .to_request();
    let resp = test::call_service(&app, users_regular).await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "regular user denied on admin users"
    );
}
