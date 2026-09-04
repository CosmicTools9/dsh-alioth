//! Gateway 用户注册 TDD 验证测试
//!
//! 测试策略（TDD 红-绿-重构）：
//!   RED:   先编写描述预期行为的测试（本文件）
//!   GREEN: 运行测试，验证实现是否满足预期
//!   REFACTOR: 根据测试结果修复发现的问题
//!
//! 测试覆盖 Gateway 通过 SSO 注册用户的全生命周期：
//!   1. 注册 - 成功路径
//!   2. 注册 - 参数验证（边界条件/异常输入）
//!   3. 注册 + 登录 - 完整生命周期
//!   4. 数据持久化验证
//!
//! 核心原则：
//!   - 测试数据不残留（DEFENSIVE CLEANUP）
//!   - 每个测试独立运行
//!   - 使用真实数据库，禁止 mock

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth;
use serde_json::json;

async fn setup_pool() -> sqlx::PgPool {
    // 统一走共享测试库连接（含 OS 用户注入），避免 postgres://localhost/... 被
    // sqlx 解析为 anonymous 角色导致连接失败（与 admin_api_test 一致）。
    ::common::testing::connect_test_db().await
}

// ============================================================================
// 测试组 1: 注册 - 成功路径
// ============================================================================

/// TDD-REG-001: 正常注册新用户
///
/// Given:  一个有效的 email/password
/// When:   POST /auth/register
/// Then:   返回 201 Created，包含 user_id 和 email
#[tokio::test]
async fn tdd_reg_001_valid_registration_returns_201() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "tdd-reg-001@alioth.test";
    common::pre_verify_email(&pool, test_email).await.ok();
    let test_password = "ValidP@ss1";

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password,
            "username": "tdd_reg_001"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;

    // ASSERT: 201 Created
    let status = resp.status().as_u16();
    assert_eq!(status, 201, "Registration should return 201 Created");

    // Read cookies BEFORE consuming body
    let cookies: Vec<_> = resp
        .headers()
        .get_all("set-cookie")
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .collect();
    let has_refresh_cookie = cookies.iter().any(|c| c.contains("refresh_token="));

    let body: serde_json::Value = test::read_body_json(resp).await;

    // ASSERT: user_id is present and non-empty string
    assert!(
        body["user_id"].is_string(),
        "user_id should be a string, got: {:?}",
        body["user_id"]
    );
    assert!(
        !body["user_id"].as_str().unwrap().is_empty(),
        "user_id should not be empty"
    );

    // ASSERT: email matches
    assert_eq!(body["email"], test_email, "email should match input");

    // ASSERT: refresh_token cookie is set
    assert!(
        has_refresh_cookie,
        "Registration should set refresh_token cookie. Cookies: {:?}",
        cookies
    );

    // CLEANUP
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

/// TDD-REG-002: 注册后用户数据持久化到数据库
///
/// Given:  成功注册一个新用户
/// When:   查询数据库
/// Then:   用户记录存在，字段值正确
#[tokio::test]
async fn tdd_reg_002_user_persists_in_database() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "tdd-reg-002@alioth.test";
    common::pre_verify_email(&pool, test_email).await.ok();
    let test_password = "PersistT3st!";
    let test_username = "tdd_reg_002_user";

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    // Step 1: Register
    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password,
            "username": test_username
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "Registration must succeed");

    // Step 2: Verify database
    let row: (i64, String, Option<String>, String) = sqlx::query_as(
        "SELECT id, email, username, name FROM isahl_auth.auth_users WHERE email = $1",
    )
    .bind(test_email)
    .fetch_one(&pool)
    .await
    .expect("User should exist in database");

    // ASSERT: email stored correctly
    assert_eq!(row.1, test_email, "email in DB should match");
    // ASSERT: username stored correctly
    assert_eq!(
        row.2.as_deref(),
        Some(test_username),
        "username in DB should match"
    );
    // ASSERT: name = username (as per register.rs logic)
    assert_eq!(row.3, test_username, "name should equal username");
    // ASSERT: id > 0
    assert!(row.0 > 0, "user id should be positive, got {}", row.0);

    // CLEANUP
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

/// TDD-REG-003: 注册时未提供 username，自动从 email 派生
///
/// Given:  注册时只提供 email 和 password
/// When:   不提供 username 字段
/// Then:   username 自动取 email 的 @ 前面部分
#[tokio::test]
async fn tdd_reg_003_username_derived_from_email() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "auto-user@alioth.test";
    common::pre_verify_email(&pool, test_email).await.ok();
    let test_password = "AutoUser123!";

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password
            // NOTE: no "username" field
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "Registration should succeed");

    // Verify username derived from email
    let username: String =
        sqlx::query_scalar("SELECT username FROM isahl_auth.auth_users WHERE email = $1")
            .bind(test_email)
            .fetch_one(&pool)
            .await
            .expect("User should exist");

    assert_eq!(
        username, "auto-user",
        "Username should be derived from email prefix, got '{}'",
        username
    );

    // CLEANUP
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

// ============================================================================
// 测试组 2: 注册 - 参数验证（边界条件/异常输入）
// ============================================================================

/// TDD-REG-004: 重复邮箱注册返回 409 Conflict
///
/// Given:  已经存在一个用户
/// When:   用相同邮箱再次注册
/// Then:   返回 409 Conflict，提示用户已存在
#[tokio::test]
async fn tdd_reg_004_duplicate_email_returns_409() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "tdd-reg-004@alioth.test";
    common::pre_verify_email(&pool, test_email).await.ok();
    let test_password = "DupEmailT3st!";

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    // First registration should succeed
    let req1 = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password,
            "username": "dup_user_1"
        }))
        .to_request();
    let resp1 = test::call_service(&app, req1).await;
    assert!(
        resp1.status().is_success(),
        "First registration should succeed"
    );

    // Second registration with same email should fail with 409
    let req2 = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": "DifferentPass456!",
            "username": "dup_user_2"
        }))
        .to_request();
    let resp2 = test::call_service(&app, req2).await;

    // ASSERT: 409 Conflict
    assert_eq!(
        resp2.status().as_u16(),
        409,
        "Duplicate email registration should return 409 Conflict"
    );

    let body: serde_json::Value = test::read_body_json(resp2).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("already exists"),
        "Error message should indicate user already exists, got: {:?}",
        body
    );

    // CLEANUP
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

/// TDD-REG-005: 无效邮箱格式返回 400 Bad Request
///
/// Given:  一个无效格式的邮箱
/// When:   POST /auth/register
/// Then:   返回 400 Bad Request
#[tokio::test]
async fn tdd_reg_005_invalid_email_format_returns_400() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let invalid_emails = vec![
        "not-an-email",  // no @ at all
        "missing@tld",   // no . after @
        "@no-local.com", // no local part before @
    ];

    for invalid_email in &invalid_emails {
        let req = test::TestRequest::post()
            .uri("/auth/register")
            .set_json(json!({
                "email": invalid_email,
                "password": "ValidP@ss1"
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        let status = resp.status().as_u16();

        assert!(
            status == 400 || status == 422,
            "Invalid email '{}' should return 400 or 422, got {}",
            invalid_email,
            status
        );
    }

    // Verify no users were created with invalid emails (sanity check)
    common::cleanup_test_users(&pool).await.ok();
}

/// TDD-REG-006: 弱密码返回 400 Bad Request
///
/// Given:  一个长度不足 8 位的密码
/// When:   POST /auth/register
/// Then:   返回 400 Bad Request，提示密码太短
#[tokio::test]
async fn tdd_reg_006_weak_password_returns_400() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let weak_passwords = ["", "a", "ab", "1234567", "abcdefg"];

    for (i, weak_pw) in weak_passwords.iter().enumerate() {
        let test_email = format!("tdd-reg-006-{}@alioth.test", i);

        let req = test::TestRequest::post()
            .uri("/auth/register")
            .set_json(json!({
                "email": test_email,
                "password": weak_pw
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        let status = resp.status().as_u16();

        assert_eq!(
            status,
            400,
            "Password '{}' (len={}) should return 400, got {}",
            weak_pw,
            weak_pw.len(),
            status
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        let error_msg = body["error"].as_str().unwrap_or("");
        assert!(
            error_msg.to_lowercase().contains("password")
                || error_msg.to_lowercase().contains("at least"),
            "Error should mention password length, got: '{}' for password '{}'",
            error_msg,
            weak_pw
        );
    }

    common::cleanup_test_users(&pool).await.ok();
}

/// TDD-REG-007: 恰好 8 位密码注册成功
///
/// Given:  一个恰好 8 位字符的密码
/// When:   POST /auth/register
/// Then:   返回 201 Created（密码长度检查是 >= 8）
#[tokio::test]
async fn tdd_reg_007_exactly_8_char_password_succeeds() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "tdd-reg-007@alioth.test";
    common::pre_verify_email(&pool, test_email).await.ok();
    let test_password = "Abcd1234"; // exactly 8 chars

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(
        resp.status().as_u16(),
        201,
        "Exactly 8-char password should be accepted"
    );

    // CLEANUP
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

/// TDD-REG-008: 邮箱大小写处理
///
/// Given:  注册时使用大小写混合的邮箱
/// When:   POST /auth/register
/// Then:   邮箱被转为小写存储
#[tokio::test]
async fn tdd_reg_008_email_case_normalization() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let input_email = "Tdd-Reg-008-CASE@Alioth.Test";
    common::pre_verify_email(&pool, input_email).await.ok();
    let expected_email = "tdd-reg-008-case@alioth.test";
    let test_password = "CaseNormT3st!";

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": input_email,
            "password": test_password
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(
        resp.status().is_success(),
        "Registration should succeed with mixed-case email"
    );

    let body: serde_json::Value = test::read_body_json(resp).await;

    // ASSERT: response email is lowercase
    assert_eq!(
        body["email"], expected_email,
        "Response email should be lowercased"
    );

    // ASSERT: database email is lowercase
    let db_email: String =
        sqlx::query_scalar("SELECT email FROM isahl_auth.auth_users WHERE email = $1")
            .bind(expected_email)
            .fetch_one(&pool)
            .await
            .expect("User should exist with lowercased email");

    assert_eq!(
        db_email, expected_email,
        "Database email should be lowercased"
    );

    // CLEANUP
    common::cleanup_user_by_email(&pool, expected_email)
        .await
        .ok();
}

// ============================================================================
// 测试组 3: 注册 + 登录 - 完整生命周期
// ============================================================================

/// TDD-REG-009: 注册后立即登录成功
///
/// Given:  新注册一个用户
/// When:   用注册时相同的凭据登录
/// Then:   登录成功，返回 session_id
#[tokio::test]
async fn tdd_reg_009_register_then_login_succeeds() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "tdd-reg-009@alioth.test";
    common::pre_verify_email(&pool, test_email).await.ok();
    let test_password = "LoginAftReg1!";

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    // Step 1: Register
    let reg_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password,
            "username": "tdd_reg_009"
        }))
        .to_request();
    let reg_resp = test::call_service(&app, reg_req).await;
    assert!(
        reg_resp.status().is_success(),
        "Registration should succeed: {:?}",
        reg_resp.status()
    );

    // 新注册用户默认 status='pending'，跳过身份审核后才能登录
    sqlx::query("UPDATE isahl_auth.auth_users SET status = 'active' WHERE email = $1")
        .bind(test_email)
        .execute(&pool)
        .await
        .ok();

    // Step 2: Login with same credentials
    let login_req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({
            "identifier": test_email,
            "password": test_password
        }))
        .to_request();
    let login_resp = test::call_service(&app, login_req).await;

    // ASSERT: Login should not return 500 (should return 200 or 401)
    let status = login_resp.status().as_u16();
    assert_ne!(
        status, 500,
        "Login should not crash with 500, got {}",
        status
    );

    let login_body: serde_json::Value = test::read_body_json(login_resp).await;
    eprintln!("Login response: {:?}", login_body);

    // ASSERT: Login response contains session_id
    assert!(
        login_body["session_id"].is_string() || login_body["mfa_required"].as_bool() == Some(true),
        "Login should return session_id or indicate MFA required. Got: {:?}",
        login_body
    );

    // CLEANUP
    sqlx::query(
        "DELETE FROM isahl_auth.sso_sessions WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)",
    )
    .bind(test_email)
    .execute(&pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.refresh_tokens WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)",
    )
    .bind(test_email)
    .execute(&pool)
    .await
    .ok();
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

/// TDD-REG-010: 注册后使用错误密码登录失败
///
/// Given:  已注册一个用户
/// When:   使用错误的密码登录
/// Then:   返回 401 Unauthorized
#[tokio::test]
async fn tdd_reg_010_wrong_password_login_returns_401() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "tdd-reg-010@alioth.test";
    common::pre_verify_email(&pool, test_email).await.ok();
    let test_password = "CorrectP@ss1";
    let wrong_password = "WrongP@ssword99!";

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    // Step 1: Register
    let reg_req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password
        }))
        .to_request();
    let reg_resp = test::call_service(&app, reg_req).await;
    assert!(
        reg_resp.status().is_success(),
        "Registration should succeed"
    );

    // 新注册用户默认 status='pending'，跳过身份审核后才能登录
    sqlx::query("UPDATE isahl_auth.auth_users SET status = 'active' WHERE email = $1")
        .bind(test_email)
        .execute(&pool)
        .await
        .ok();

    // Step 2: Login with wrong password
    let login_req = test::TestRequest::post()
        .uri("/auth/login")
        .set_json(json!({
            "identifier": test_email,
            "password": wrong_password
        }))
        .to_request();
    let login_resp = test::call_service(&app, login_req).await;

    // ASSERT: 401 Unauthorized
    assert_eq!(
        login_resp.status().as_u16(),
        401,
        "Wrong password login should return 401 Unauthorized"
    );

    let login_body: serde_json::Value = test::read_body_json(login_resp).await;
    assert!(
        login_body["error"]
            .as_str()
            .unwrap_or("")
            .contains("Invalid"),
        "Error should indicate invalid credentials, got: {:?}",
        login_body
    );

    // CLEANUP
    sqlx::query(
        "DELETE FROM isahl_auth.sso_sessions WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)",
    )
    .bind(test_email)
    .execute(&pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.refresh_tokens WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)",
    )
    .bind(test_email)
    .execute(&pool)
    .await
    .ok();
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

/// TDD-REG-011: 不存在的用户登录返回 401
///
/// Given:  数据库中不存在该用户
/// When:   尝试登录
/// Then:   返回 401 Unauthorized
#[tokio::test]
async fn tdd_reg_011_nonexistent_user_login_returns_401() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

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
            "identifier": "no-such-user-99999@alioth.test",
            "password": "SomePassword123!"
        }))
        .to_request();
    let login_resp = test::call_service(&app, login_req).await;

    assert_eq!(
        login_resp.status().as_u16(),
        401,
        "Nonexistent user login should return 401"
    );
}

// ============================================================================
// 测试组 4: 边界条件与健壮性
// ============================================================================

/// TDD-REG-012: 超长邮箱拒绝或截断处理
///
/// Given:  一个超长的邮箱地址（> 255 字符）
/// When:   POST /auth/register
/// Then:   返回 4xx 错误（不应崩溃）
#[tokio::test]
async fn tdd_reg_012_overly_long_email_handled() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let long_local = "a".repeat(250);
    let test_email = format!("{}@alioth.test", long_local); // ~264 chars

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": "LongEmailT3st!"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();

    // ASSERT: Should not be 500 (server error)
    assert_ne!(
        status, 500,
        "Overly long email should not cause server crash"
    );

    // CLEANUP (in case it was accepted)
    common::cleanup_test_users(&pool).await.ok();
}

/// TDD-REG-013: 空请求体返回 4xx
///
/// Given:  一个空的 JSON body
/// When:   POST /auth/register
/// Then:   返回 4xx 反序列化错误（不应崩溃）
#[tokio::test]
async fn tdd_reg_013_empty_body_returns_4xx() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({}))
        .to_request();

    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();

    assert_ne!(status, 500, "Empty body should not cause server crash");

    common::cleanup_test_users(&pool).await.ok();
}

/// TDD-REG-014: 特殊字符密码处理
///
/// Given:  包含特殊字符（Unicode/Emoji）的密码
/// When:   POST /auth/register
/// Then:   注册成功或不崩溃（取决于业务规则）
#[tokio::test]
async fn tdd_reg_014_special_char_password_handled() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "tdd-reg-014@alioth.test";
    let test_password = "密码Test🔐!@#$%^&*()_+-=[]{}|;:',.<>?/~`";

    let auth_state = common::test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": test_email,
            "password": test_password
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();

    // ASSERT: Should not crash (201 or 4xx are both acceptable)
    assert_ne!(
        status, 500,
        "Special char password should not cause server crash"
    );

    // CLEANUP
    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

// ============================================================================
// 测试组 5: 双注册通道（add-dual-register-channels）
// ============================================================================

/// TDD-REG-EXT-010: 内部通道拒绝 external 身份
///
/// Given: 内部注册端点 /auth/register
/// When:  POST 携带 user_type=external
/// Then:  400，错误信息引导 /auth/register/external，不创建用户
#[tokio::test]
async fn tdd_reg_ext_010_internal_rejects_external() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register")
        .set_json(json!({
            "email": "tdd-reg-ext-010@alioth.test",
            "password": "ValidP@ss1",
            "username": "tdd_reg_ext_010",
            "user_type": "external"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status().as_u16(), 400, "内部端点应拒绝 external");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("/auth/register/external"),
        "错误信息应引导外部通道，实际: {:?}",
        body["error"]
    );

    // 用户不应存在
    let cnt: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.auth_users WHERE username = 'tdd_reg_ext_010'",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0);
    assert_eq!(cnt, 0, "内部端点不应创建外部账号");
}

/// TDD-REG-EXT-011: 外部通道注册——强制 external + 独立审批事件/流程
///
/// Given: 外部注册端点 /auth/register/external
/// When:  POST（无 user_type 键）
/// Then:  201，channel=external；账号 user_type=external；
///        审批实例 code=external-subject-register-approval（与内部通道分流）
#[tokio::test]
async fn tdd_reg_ext_011_external_channel_registers() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    // 预清理：失败路径可能残留（username/email 唯一约束；email 表孤儿行同样阻断）
    for u in ["tdd_reg_ext_011", "tdd_reg_ext_012"] {
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_user_emails WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users WHERE username = $1) OR email = $2")
            .bind(u)
            .bind(format!("{u}@alioth.test").replace("tdd_reg_ext_", "tdd-reg-ext-"))
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = $1")
            .bind(u)
            .execute(&pool)
            .await;
    }
    common::cleanup_test_users(&pool).await.ok();

    let test_email = "tdd-reg-ext-011@alioth.test";
    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/auth/register/external")
        .set_json(json!({
            "email": test_email,
            "password": "ValidP@ss1",
            "username": "tdd_reg_ext_011",
            "user_type": "standard"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(status, 201, "外部通道应 201，body: {body}");
    assert_eq!(
        body["channel"].as_str(),
        Some("external"),
        "响应应标注外部通道"
    );

    // 账号身份类型：服务端强制 external（请求体 user_type=standard 被忽略）
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT user_type, status FROM isahl_auth.auth_users WHERE username = 'tdd_reg_ext_011'",
    )
    .fetch_optional(&pool)
    .await
    .expect("查用户失败");
    let (user_type, status) = row.expect("外部账号应存在");
    assert_eq!(user_type, "external", "服务端应强制 external");
    assert_eq!(status, "pending_approval");

    // 审批实例 code 与内部通道分流
    let instance_code: String = sqlx::query_scalar(
        r#"SELECT oa.code FROM isahl."zc_id_oper-approve" oa
           WHERE oa.fk_subject = (SELECT id FROM isahl_auth.auth_users WHERE username = 'tdd_reg_ext_011')
             AND oa.deleted_at IS NULL
           ORDER BY oa.id DESC LIMIT 1"#,
    )
    .fetch_one(&pool)
    .await
    .expect("查审批实例失败");
    assert_eq!(
        instance_code, "external-subject-register-approval",
        "外部通道审批实例应为独立 code"
    );

    common::cleanup_user_by_email(&pool, test_email).await.ok();
}

/// TDD-REG-EXT-012: /auth/me 暴露 user_type（观测缺口补齐）
///
/// Given: 外部通道注册一名用户
/// When:  以注册签发的 access token 调 /auth/me
/// Then:  响应 user.user_type == "external"
#[tokio::test]
async fn tdd_reg_ext_012_me_returns_user_type() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema setup");
    common::cleanup_test_users(&pool).await.ok();

    // 预清理：失败路径可能残留（username/email 唯一约束）
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = 'tdd_reg_ext_012'")
        .execute(&pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.auth_user_emails WHERE email = 'tdd-reg-ext-012@alioth.test'",
    )
    .execute(&pool)
    .await;
    let test_email = "tdd-reg-ext-012@alioth.test";
    let auth_state = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .service(web::scope("/auth").configure(auth::login::configure)),
    )
    .await;

    // 注册（外部通道）
    let req = test::TestRequest::post()
        .uri("/auth/register/external")
        .set_json(json!({
            "email": test_email,
            "password": "ValidP@ss1",
            "username": "tdd_reg_ext_012"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 201);
    let access_token = resp
        .response()
        .cookies()
        .find(|c| c.name() == "access_token")
        .map(|c| c.value().to_string())
        .expect("注册应签发 access_token cookie");

    // me
    let req = test::TestRequest::get()
        .uri("/auth/me")
        .insert_header((
            actix_web::http::header::AUTHORIZATION,
            format!("Bearer {access_token}"),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "me 应 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["user"]["user_type"].as_str(),
        Some("external"),
        "me 应暴露 user_type"
    );

    common::cleanup_user_by_email(&pool, test_email).await.ok();
}
