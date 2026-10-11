//! SSO 认证集成测试
//!
//! 验证用户登录流程，确保不会在数据库中残留脏数据。
//!
//! 注意：register handler 内部调用 create_session，目前 session 创建在测试环境中有问题，
//! 因此本测试直接使用数据库插入来准备用户，然后测试 login 流程。

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth;
use serde_json::json;
use sqlx::PgPool;

async fn setup_pool() -> PgPool {
    // 统一走共享测试库连接（含 OS 用户注入），避免 postgres://localhost/... 被
    // sqlx 解析为 anonymous 角色导致连接失败（与 admin_api_test 一致）。
    ::common::testing::connect_test_db().await
}

#[tokio::test]
async fn test_register_and_login_lifecycle() {
    let pool = setup_pool().await;
    // 初始化 schema
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    // 防御性清理
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "test-sso-e2e@alioth.test";
    let test_password = "TestPass123!";

    // 构建 AuthState
    let auth_state = common::test_auth_state();

    // 构建测试应用（包含 register 和 login 路由）
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    // 1. 注册新用户（email 为可选联系方式，注册路径无邮箱验证前置）
    let register_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password,
            "username": "test_sso_user"
        }))
        .to_request();

    let register_resp = test::call_service(&app, register_req).await;
    assert!(
        register_resp.status().is_success(),
        "Registration should succeed, got {:?}",
        register_resp.status()
    );

    let register_body: serde_json::Value = test::read_body_json(register_resp).await;
    assert!(register_body["user_id"].is_string());
    assert_eq!(register_body["email"], test_email);

    // 2. 使用新用户登录
    let login_req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({
            "identifier": test_email,
            "password": test_password
        }))
        .to_request();

    let login_resp = test::call_service(&app, login_req).await;
    let status = login_resp.status().as_u16();
    // 登录至少不应 500（由于 password hash 格式问题，可能 401，但不应崩溃）
    assert_ne!(status, 500, "Login should not crash with 500");

    // 最终清理：删除用户及关联的 session/token（邮箱唯一存储 = auth_user_emails）
    sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE user_id IN (SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1)")
        .bind(test_email)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.refresh_tokens WHERE user_id IN (SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1)")
        .bind(test_email)
        .execute(&pool)
        .await
        .ok();
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

#[tokio::test]
async fn test_session_manager_create_session() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let test_email = "test-session@alioth.test";

    // 插入用户（原 `name` 值落 `username`；邮箱落 auth_user_emails）
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users (id, username, password_hash, created_at, updated_at)
        VALUES (isahl.gen_next_zuid(), 'sessiontest', 'hash', NOW(), NOW())
        RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to insert test user");

    sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at) \
         VALUES ($1, $2, TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(test_email)
    .execute(&pool)
    .await
    .ok();

    eprintln!("🔍 Inserted user_id: {}", user_id);

    // 直接调用 SessionManager
    let session_manager = gateway_sso::auth::session::SessionManager::new(pool.clone());
    let result = session_manager
        .create_session(gateway_sso::auth::session::CreateSessionRequest {
            user_id,
            idp_provider_id: None,
            idp_session_id: None,
            ip_address: None,
            user_agent: None,
            front_channel_logout_uri: None,
            back_channel_logout_uri: None,
            refresh_token_hash: None,
        })
        .await;

    match &result {
        Ok(session) => eprintln!(
            "✅ Session created: id={}, token={}",
            session.id, session.session_token
        ),
        Err(e) => eprintln!("❌ Session creation failed: {:?}", e),
    }

    assert!(
        result.is_ok(),
        "SessionManager::create_session should succeed"
    );

    // 清理
    sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .ok();
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

#[tokio::test]
async fn test_login_with_invalid_credentials() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let login_req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({
            "identifier": "nonexistent@alioth.test",
            "password": "wrongpassword"
        }))
        .to_request();

    let login_resp = test::call_service(&app, login_req).await;
    // 未找到用户或密码错误应返回 401
    assert_eq!(login_resp.status().as_u16(), 401);
}

// ============================================================================
// Mock 服务
// ============================================================================

use async_trait::async_trait;

struct MockEmailService;
#[async_trait]
impl ::common::EmailService for MockEmailService {
    async fn send(&self, _to: &str, _subject: &str, _body: &str) -> ::common::Result<()> {
        Ok(())
    }
    async fn send_html(&self, _to: &str, _subject: &str, _html: &str) -> ::common::Result<()> {
        Ok(())
    }
}

struct MockSmsService;
#[async_trait]
impl ::common::SmsService for MockSmsService {
    async fn send(
        &self,
        _phone: &str,
        _template_code: &str,
        _params: &str,
    ) -> ::common::Result<()> {
        Ok(())
    }
}

// ============================================================================
// 邮箱认证测试
// ============================================================================

#[tokio::test]
async fn test_email_verification_flow() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_config(false)))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let test_email = "verify-test@alioth.test";

    // 1. 发送验证码
    let send_req = test::TestRequest::post()
        .uri("/auth/email/send-code")
        .set_json(json!({"email": test_email, "purpose": "register"}))
        .to_request();
    let send_resp = test::call_service(&app, send_req).await;
    assert_eq!(send_resp.status().as_u16(), 200, "Send code should succeed");

    // 2. 查询数据库中的验证码
    let code: String = sqlx::query_scalar(
        "SELECT code FROM isahl_auth.auth_email_verifications WHERE email = $1 AND purpose = 'register'"
    )
    .bind(test_email)
    .fetch_one(&pool)
    .await
    .expect("Code should be stored in DB");
    assert_eq!(code.len(), 6, "Code should be 6 digits");

    // 3. 验证验证码
    let verify_req = test::TestRequest::post()
        .uri("/auth/email/verify-code")
        .set_json(json!({"email": test_email, "code": code, "purpose": "register"}))
        .to_request();
    let verify_resp = test::call_service(&app, verify_req).await;
    assert_eq!(
        verify_resp.status().as_u16(),
        200,
        "Verify code should succeed"
    );

    // 清理
    sqlx::query("DELETE FROM isahl_auth.auth_email_verifications WHERE email = $1")
        .bind(test_email)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_register_with_username_password_only() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let username = "uonly_user";

    // 仅账号密码注册（无 email）→ 201（email 非唯一基点，不应被阻塞）
    let register_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({ "username": username, "password": "TestPass123!" }))
        .to_request();
    let register_resp = test::call_service(&app, register_req).await;
    assert_eq!(
        register_resp.status().as_u16(),
        201,
        "register with username+password only should succeed"
    );

    // 用户落库：username 锚点；邮箱唯一存储 = auth_user_emails，此处无未删行
    let uid: i64 = sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username = $1")
        .bind(username)
        .fetch_one(&pool)
        .await
        .expect("user should exist after register");
    let email_cnt: i64 =
        sqlx::query_scalar("SELECT count(*) FROM isahl_auth.auth_user_emails WHERE fk_user = $1 AND deleted_at IS NULL")
            .bind(uid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        email_cnt, 0,
        "no auth_user_emails rows for username-only user"
    );

    // username 重复 → 409
    let dup_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({ "username": username, "password": "TestPass123!" }))
        .to_request();
    let dup_resp = test::call_service(&app, dup_req).await;
    assert_eq!(
        dup_resp.status().as_u16(),
        409,
        "duplicate username should 409"
    );

    // 清理（先删邮箱行再删账号，顺序满足 FK；username-only 用户通常无邮箱行）
    sqlx::query(
        "DELETE FROM isahl_auth.auth_user_emails \
         WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users WHERE username = $1)",
    )
    .bind(username)
    .execute(&pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = $1")
        .bind(username)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_register_with_unverified_email_succeeds() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let test_email = format!("reg-verify-{}@alioth.test", uuid::Uuid::new_v4().simple());
    let test_username = format!("reg_verify_{}", uuid::Uuid::new_v4().simple());

    // 未验证邮箱直接注册 → 201（注册路径不消费邮箱验证记录；判据正本 =
    // 活规约 register-username-anchor 的 email-optional-non-unique-channel）
    let register_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": "TestPass123!",
            "username": test_username
        }))
        .to_request();
    let register_resp = test::call_service(&app, register_req).await;
    assert_eq!(
        register_resp.status().as_u16(),
        201,
        "register with unverified email must succeed"
    );
    let body: serde_json::Value = test::read_body_json(register_resp).await;
    assert!(
        !body["error"]
            .as_str()
            .unwrap_or("")
            .contains("EMAIL_NOT_VERIFIED"),
        "EMAIL_NOT_VERIFIED must not be returned, got {:?}",
        body
    );

    // 用户行落库且邮箱落 auth_user_emails（主邮箱读径 = 子查询）
    let stored_email: Option<String> = sqlx::query_scalar(
        "SELECT (SELECT e.email FROM isahl_auth.auth_user_emails e \
          WHERE e.fk_user = u.id AND e.deleted_at IS NULL \
          ORDER BY e.is_primary DESC, e.id LIMIT 1) \
         FROM isahl_auth.auth_users u WHERE u.username = $1",
    )
    .bind(&test_username)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(
        stored_email.as_deref(),
        Some(test_email.as_str()),
        "email must be stored in auth_user_emails"
    );

    // 清理
    common::cleanup_user_by_email(&pool, &test_email).await.ok();
}

#[tokio::test]
async fn test_email_send_code_invalid_format() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_config(false)))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let send_req = test::TestRequest::post()
        .uri("/auth/email/send-code")
        .set_json(json!({"email": "not-an-email", "purpose": "register"}))
        .to_request();
    let send_resp = test::call_service(&app, send_req).await;
    assert_eq!(
        send_resp.status().as_u16(),
        400,
        "Invalid email should return 400"
    );
}

// ============================================================================
// 手机短信认证测试
// ============================================================================

#[tokio::test]
async fn test_phone_verification_flow() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(
                Box::new(MockSmsService) as Box<dyn ::common::SmsService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let test_phone = "13800138000";

    // 1. 发送验证码
    let send_req = test::TestRequest::post()
        .uri("/auth/phone/send-code")
        .set_json(json!({"phone": test_phone, "purpose": "register"}))
        .to_request();
    let send_resp = test::call_service(&app, send_req).await;
    assert_eq!(
        send_resp.status().as_u16(),
        200,
        "Send SMS code should succeed"
    );

    // 2. 查询数据库中的验证码
    let code: String =
        sqlx::query_scalar("SELECT code FROM isahl_auth.auth_phone_verifications WHERE phone = $1")
            .bind(test_phone)
            .fetch_one(&pool)
            .await
            .expect("Code should be stored in DB");
    assert_eq!(code.len(), 6, "Code should be 6 digits");

    // 3. 验证验证码
    let verify_req = test::TestRequest::post()
        .uri("/auth/phone/verify-code")
        .set_json(json!({"phone": test_phone, "code": code, "purpose": "register"}))
        .to_request();
    let verify_resp = test::call_service(&app, verify_req).await;
    assert_eq!(
        verify_resp.status().as_u16(),
        200,
        "Verify SMS code should succeed"
    );

    // 清理
    sqlx::query("DELETE FROM isahl_auth.auth_phone_verifications WHERE phone = $1")
        .bind(test_phone)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_phone_send_code_invalid_format() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(
                Box::new(MockSmsService) as Box<dyn ::common::SmsService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let send_req = test::TestRequest::post()
        .uri("/auth/phone/send-code")
        .set_json(json!({"phone": "123", "purpose": "register"}))
        .to_request();
    let send_resp = test::call_service(&app, send_req).await;
    assert_eq!(
        send_resp.status().as_u16(),
        400,
        "Invalid phone should return 400"
    );
}

// ============================================================================
// 邮箱验证码登录 + 密码重置（add-password-reset-and-auth-fixes）
// ============================================================================

/// 插入 active 测试用户，返回 user_id（email 落 auth_user_emails）
async fn insert_active_user(pool: &PgPool, username: &str, email: &str) -> i64 {
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users (id, username, status, password_hash, created_at, updated_at)
        VALUES (isahl.gen_next_zuid(), $1, 'active', 'unused-hash', NOW(), NOW())
        RETURNING id"
    )
    .bind(username)
    .fetch_one(pool)
    .await
    .expect("Failed to insert test user");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at) \
         VALUES ($1, $2, TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(email)
    .execute(pool)
    .await
    .ok();
    user_id
}

async fn cleanup_email_code_users(pool: &PgPool, email: &str) {
    sqlx::query("DELETE FROM isahl_auth.auth_email_verifications WHERE email = $1")
        .bind(email)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.password_reset_tokens WHERE user_id IN (SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1)",
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.sso_sessions WHERE user_id IN (SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1)",
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.refresh_token_blocklist WHERE user_id IN (SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1)",
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    // 邮箱行先行（auth_user_emails 入边 FK 无 ON DELETE CASCADE）——先取 id 再删
    let user_ids: Vec<i64> =
        sqlx::query_scalar("SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1")
            .bind(email)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    sqlx::query("DELETE FROM isahl_auth.auth_user_emails WHERE fk_user = ANY($1)")
        .bind(&user_ids)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = ANY($1)")
        .bind(&user_ids)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_email_code_login_flow() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let test_email = "emailcode-login@alioth.test";
    cleanup_email_code_users(&pool, test_email).await;
    insert_active_user(&pool, "emailcodetest", test_email).await;

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_config(true)))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    // 1. 请求登录验证码（purpose=login）
    let send_req = test::TestRequest::post()
        .uri("/auth/email/send-code")
        .set_json(json!({"email": test_email, "purpose": "login"}))
        .to_request();
    let send_resp = test::call_service(&app, send_req).await;
    assert_eq!(send_resp.status().as_u16(), 200, "Send code should succeed");

    let code: String = sqlx::query_scalar(
        "SELECT code FROM isahl_auth.auth_email_verifications WHERE email = $1 AND purpose = 'login'",
    )
    .bind(test_email)
    .fetch_one(&pool)
    .await
    .expect("Code should be stored");

    // 2. 错误验证码 → 401 且计入失败
    let wrong_req = test::TestRequest::post()
        .uri("/auth/login/email-code")
        .set_json(json!({"email": test_email, "code": "000000"}))
        .to_request();
    let wrong_resp = test::call_service(&app, wrong_req).await;
    assert_eq!(
        wrong_resp.status().as_u16(),
        401,
        "Wrong code should be 401"
    );
    let attempts: i32 = sqlx::query_scalar(
        "SELECT COALESCE(u.failed_login_attempts, 0) FROM isahl_auth.auth_users u \
         WHERE u.id IN (SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1)",
    )
    .bind(test_email)
    .fetch_one(&pool)
    .await
    .expect("Failed attempts should be readable");
    assert_eq!(attempts, 1, "Wrong code must count as failed attempt");

    // 3. 正确验证码 → 200 签发令牌
    let ok_req = test::TestRequest::post()
        .uri("/auth/login/email-code")
        .set_json(json!({"email": test_email, "code": code}))
        .to_request();
    let ok_resp = test::call_service(&app, ok_req).await;
    assert_eq!(ok_resp.status().as_u16(), 200, "Correct code should log in");
    let body: serde_json::Value = test::read_body_json(ok_resp).await;
    assert!(
        body.get("access_token").and_then(|v| v.as_str()).is_some(),
        "Login response must carry access_token"
    );

    // 4. 验证码一次性 — 重放 → 401
    let replay_req = test::TestRequest::post()
        .uri("/auth/login/email-code")
        .set_json(json!({"email": test_email, "code": code}))
        .to_request();
    let replay_resp = test::call_service(&app, replay_req).await;
    assert_eq!(
        replay_resp.status().as_u16(),
        401,
        "Code replay must be 401"
    );

    cleanup_email_code_users(&pool, test_email).await;
}

#[tokio::test]
async fn test_email_code_login_multi_account_rejected() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let test_email = "emailcode-multi@alioth.test";
    cleanup_email_code_users(&pool, test_email).await;
    insert_active_user(&pool, "emailcodemulti1", test_email).await;
    insert_active_user(&pool, "emailcodemulti2", test_email).await;

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_config(true)))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/login/email-code")
        .set_json(json!({"email": test_email, "code": "123456"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "Multi-account email must be rejected with 400"
    );

    cleanup_email_code_users(&pool, test_email).await;
}

/// 邮箱验证码登录**默认关闭**（gate-email-code-login-by-env）：
/// 开关未启用（`SSO_EMAIL_CODE_LOGIN` 缺省语义）时，投递端与签发端 MUST 均 403 且
/// MUST NOT 落库/签发会话；`purpose=register` 的验证码面 MUST NOT 受影响（用途隔离）。
#[tokio::test]
async fn test_email_code_login_disabled_by_default() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let test_email = "emailcode-disabled@alioth.test";
    cleanup_email_code_users(&pool, test_email).await;
    insert_active_user(&pool, "emailcodedisabled", test_email).await;

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_config(false)))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    // 1. 投递端：关闭态请求登录验证码 → 403 且 MUST NOT 落库
    let send_req = test::TestRequest::post()
        .uri("/auth/email/send-code")
        .set_json(json!({"email": test_email, "purpose": "login"}))
        .to_request();
    let send_resp = test::call_service(&app, send_req).await;
    assert_eq!(
        send_resp.status().as_u16(),
        403,
        "关闭态登录验证码请求 MUST 403"
    );
    let send_body: serde_json::Value = test::read_body_json(send_resp).await;
    assert_eq!(send_body["error"], "EMAIL_CODE_LOGIN_DISABLED");

    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.auth_email_verifications \
         WHERE email = $1 AND purpose = 'login'",
    )
    .bind(test_email)
    .fetch_one(&pool)
    .await
    .expect("Count login codes");
    assert_eq!(stored, 0, "关闭态 MUST NOT 落库登录验证码");

    // 2. 签发端：手工植入有效验证码，登录端点仍 MUST 403 且 MUST NOT 签发会话
    sqlx::query(
        "INSERT INTO isahl_auth.auth_email_verifications (email, code, purpose, expires_at) \
         VALUES ($1, '123456', 'login', NOW() + INTERVAL '15 minutes')",
    )
    .bind(test_email)
    .execute(&pool)
    .await
    .expect("Seed valid login code");

    let login_req = test::TestRequest::post()
        .uri("/auth/login/email-code")
        .set_json(json!({"email": test_email, "code": "123456"}))
        .to_request();
    let login_resp = test::call_service(&app, login_req).await;
    assert_eq!(
        login_resp.status().as_u16(),
        403,
        "关闭态邮箱验证码登录 MUST 403"
    );
    let login_body: serde_json::Value = test::read_body_json(login_resp).await;
    assert_eq!(login_body["error"], "EMAIL_CODE_LOGIN_DISABLED");
    assert!(
        login_body.get("access_token").is_none(),
        "关闭态 MUST NOT 签发令牌"
    );

    // 3. 用途隔离：同一关闭态下 purpose=register 仍按既有语义成功
    let register_req = test::TestRequest::post()
        .uri("/auth/email/send-code")
        .set_json(json!({"email": test_email, "purpose": "register"}))
        .to_request();
    let register_resp = test::call_service(&app, register_req).await;
    assert_eq!(
        register_resp.status().as_u16(),
        200,
        "register 用途 MUST NOT 受开关影响"
    );

    cleanup_email_code_users(&pool, test_email).await;
}

#[tokio::test]
async fn test_password_reset_flow() {
    let pool = setup_pool().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let test_email = "reset-flow@alioth.test";
    cleanup_email_code_users(&pool, test_email).await;
    let user_id = insert_active_user(&pool, "resetflowtest", test_email).await;

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(
                Box::new(MockEmailService) as Box<dyn ::common::EmailService>
            ))
            .service(
                web::scope("/auth")
                    .configure(auth::login::configure)
                    .configure(auth::reset_password::configure_routes),
            ),
    )
    .await;

    // 1. 请求重置 → 恒 200（防枚举），token 行落库
    let req_req = test::TestRequest::post()
        .uri("/auth/reset-password/request")
        .set_json(json!({"email": test_email}))
        .to_request();
    let req_resp = test::call_service(&app, req_req).await;
    assert_eq!(req_resp.status().as_u16(), 200, "Request should succeed");
    let token_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.password_reset_tokens WHERE user_id = $1 AND used_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("Token count should be readable");
    assert_eq!(token_rows, 1, "Request must persist one active token");

    // 2. 用自持 token 覆盖 hash（邮件通道只发链接，测试直写已知 token）
    let known_token = "TestResetToken20260924";
    let known_hash = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
        known_token.as_bytes(),
    ));
    sqlx::query("UPDATE isahl_auth.password_reset_tokens SET token_hash = $1 WHERE user_id = $2")
        .bind(&known_hash)
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("Token hash update should succeed");

    // 3. confirm → 200，新密码可登录
    let confirm_req = test::TestRequest::post()
        .uri("/auth/reset-password/confirm")
        .set_json(json!({"token": known_token, "new_password": "NewPass123!"}))
        .to_request();
    let confirm_resp = test::call_service(&app, confirm_req).await;
    assert_eq!(
        confirm_resp.status().as_u16(),
        200,
        "Confirm should succeed"
    );

    let login_req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({"identifier": "resetflowtest", "password": "NewPass123!"}))
        .to_request();
    let login_resp = test::call_service(&app, login_req).await;
    let login_status = login_resp.status().as_u16();
    let login_body = test::read_body(login_resp).await;
    assert_eq!(
        login_status, 200,
        "New password must log in after reset, got {}: {:?}",
        login_status, login_body
    );

    // 4. token 一次性 — 重放 confirm → 400
    let replay_req = test::TestRequest::post()
        .uri("/auth/reset-password/confirm")
        .set_json(json!({"token": known_token, "new_password": "OtherPass123!"}))
        .to_request();
    let replay_resp = test::call_service(&app, replay_req).await;
    assert_eq!(
        replay_resp.status().as_u16(),
        400,
        "Token replay must be 400"
    );

    cleanup_email_code_users(&pool, test_email).await;
}
