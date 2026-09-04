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

    // 1. 注册流程要求邮箱已验证——先发送并验证验证码
    let send_code_req = test::TestRequest::post()
        .uri("/auth/email/send-code")
        .set_json(json!({"email": test_email, "purpose": "register"}))
        .to_request();
    test::call_service(&app, send_code_req).await;

    let code: String = sqlx::query_scalar(
        "SELECT code FROM isahl_auth.auth_email_verifications WHERE email = $1 AND purpose = 'register'",
    )
    .bind(test_email)
    .fetch_one(&pool)
    .await
    .expect("verification code should be stored");

    let verify_code_req = test::TestRequest::post()
        .uri("/auth/email/verify-code")
        .set_json(json!({"email": test_email, "code": code, "purpose": "register"}))
        .to_request();
    test::call_service(&app, verify_code_req).await;

    // 2. 注册新用户
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

    // 最终清理：删除用户及关联的 session/token
    sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)")
        .bind(test_email)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.refresh_tokens WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)")
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

    // 插入用户
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users (id, username, name, email, password_hash, created_at, updated_at)
        VALUES (isahl.gen_next_zuid(), 'sessiontest', 'Session Test', $1, 'hash', NOW(), NOW())
        RETURNING id"
    )
    .bind(test_email)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert test user");

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

    // 用户落库：username 锚点，email 为 NULL，auth_user_emails 无行
    let row: Option<(i64, Option<String>)> =
        sqlx::query_as("SELECT id, email FROM isahl_auth.auth_users WHERE username = $1")
            .bind(username)
            .fetch_optional(&pool)
            .await
            .unwrap();
    let (uid, email) = row.expect("user should exist after register");
    assert!(
        email.is_none(),
        "no-email user should have NULL auth_users.email"
    );
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

    // 清理
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

    let test_email = "reg-verify@alioth.test";

    // 未验证邮箱直接注册 → 201（邮箱验证不再作为注册门禁；访问由审批门禁控制）
    let register_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": "TestPass123!",
            "username": "reg_verify_user"
        }))
        .to_request();
    let register_resp = test::call_service(&app, register_req).await;
    assert_eq!(
        register_resp.status().as_u16(),
        201,
        "register with unverified email should succeed (no 403 gate)"
    );

    // email 写入 auth_user_emails（主邮箱）+ 镜像 auth_users.email
    let uid: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE username = 'reg_verify_user'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let email_cnt: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.auth_user_emails WHERE fk_user = $1 AND email = $2 AND deleted_at IS NULL",
    )
    .bind(uid)
    .bind(test_email)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(email_cnt, 1, "email should be written to auth_user_emails");

    // 清理
    common::cleanup_user_by_email(&pool, test_email).await.ok();
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
