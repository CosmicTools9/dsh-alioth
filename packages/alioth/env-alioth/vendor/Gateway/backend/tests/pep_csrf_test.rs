//! Gateway PEP CSRF（double-submit）集成测试 —— change `implement-csrf-protection`。
//!
//! 覆盖（design §4/§6 验证清单；全部走真实 PEP 中间件，非单元 stub）：
//!   1. cookie 鉴权 + 无 `X-CSRF-Token` ⇒ 403 平铺体 `CSRF_TOKEN_MISSING`；
//!   2. cookie 鉴权 + 错 `X-CSRF-Token` ⇒ 403 平铺体 `CSRF_TOKEN_MISMATCH`；
//!   3. cookie 鉴权 + 正确头 ⇒ 通过（正向路径不被门禁误伤）；
//!   4. Bearer 鉴权（无 csrf 头）⇒ 通过（攻击者无法跨站注入自定义头 ⇒ 非 CSRF 面）；
//!   5. 安全方法（GET）⇒ 不校验；
//!   6. 次序回归：失效会话（会话已吊销）写请求 MUST 仍 401，不得被 CSRF 403 抢先；
//!      令牌解码失败同理仍 401。
//!   7. 账号态复查（E2 / 评审候选 2 的 PEP 半）：会话行有效但主体账号被禁用 ⇒ 401
//!      （`is_session_active` join `auth_users.is_active`；red-before/green-after 判据）。
//!
//! 夹具：自持 ES256 密钥对（固定标量派生——零内嵌密钥材料，与生产/其他夹具不共源）；
//! PDP 决策走 `NGAC_FAIL_OPEN=true`（dev/standalone 交付语义）⇒ 无需 SSO stub 即可观测
//! 「CSRF 门之后」的放行；真实测试库仅用于会话吊销校验（`isahl_auth.sso_sessions`）。

use actix_web::{test, web, App, HttpResponse};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use sqlx::PgPool;

use ::common::testing::connect_test_db;
use alioth_gateway::pep::NgacEnforcer;

/// PEP 的 iss/aud 绑定值（`with_token_binding`，与测试令牌签发值一致）。
const TEST_ISSUER: &str = "http://localhost:9002";

/// 测试私钥 PEM（由固定标量派生：确定性、无内嵌密钥材料；生产路径 MUST NOT 引用）。
fn test_private_key_pem() -> String {
    use p256::elliptic_curve::pkcs8::EncodePrivateKey;
    p256::SecretKey::from_slice(&[9u8; 32])
        .expect("deterministic test scalar must be a valid P-256 scalar")
        .to_pkcs8_pem(p256::elliptic_curve::pkcs8::LineEnding::LF)
        .expect("test key PEM encode")
        .to_string()
}

/// 与 [`test_private_key_pem`] 配对的公钥 PEM（PEP 静态验签钥回退通道）。
fn test_public_key_pem() -> Vec<u8> {
    use p256::elliptic_curve::pkcs8::EncodePublicKey;
    p256::SecretKey::from_slice(&[9u8; 32])
        .expect("deterministic test scalar must be a valid P-256 scalar")
        .public_key()
        .to_public_key_pem(p256::elliptic_curve::pkcs8::LineEnding::LF)
        .expect("test public key PEM encode")
        .into_bytes()
}

/// 签发测试 JWT（ES256）。`sid` 非空 ⇒ PEP 执行会话吊销校验（需 pool；本文件用真实测试库）。
fn issue_token(sub: &str, sid: &str) -> String {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = json!({
        "sub": sub,
        "exp": now + 3600,
        "iat": now,
        "email": format!("{sub}@alioth.test"),
        "username": sub,
        "sid": sid,
        "iss": TEST_ISSUER,
        "aud": TEST_ISSUER,
        "scope": "",
        "svc_user_id": 0,
    });
    encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(test_private_key_pem().as_bytes()).expect("test ec key"),
    )
    .expect("sign test token")
}

/// 构造 PEP：静态公钥 = 测试公钥，iss/aud 绑定 = [`TEST_ISSUER`]，无 PDP 端点
/// （决策走 `NGAC_FAIL_OPEN`；pool 提供会话吊销校验与审计落库）。
fn enforcer(pool: &PgPool) -> NgacEnforcer {
    NgacEnforcer::new(
        pool.clone(),
        test_public_key_pem(),
        Vec::new(),
        String::new(),
    )
    .with_token_binding(TEST_ISSUER.to_string(), TEST_ISSUER.to_string())
}

/// 用例公共环境：CSRF 开关打开 + fail-open（无 PDP 时放行）。env 读点均为请求时。
fn arm_env() {
    std::env::set_var("CSRF_ENFORCE", "on");
    std::env::set_var("NGAC_FAIL_OPEN", "true");
}

async fn ok_handler() -> HttpResponse {
    HttpResponse::Ok().json(json!({ "ok": true }))
}

/// ① cookie 鉴权 + 无 CSRF 头 ⇒ 403 `CSRF_TOKEN_MISSING`（平铺体）。
#[actix_web::test]
async fn cookie_write_without_csrf_header_is_403_missing() {
    arm_env();
    let pool = connect_test_db().await;
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(enforcer(&pool))
                .route("/csrf-probe", web::post().to(ok_handler)),
        ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/api/csrf-probe")
        .cookie(actix_web::cookie::Cookie::new(
            "access_token",
            issue_token("42", ""),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "cookie 鉴权写请求无 CSRF 头 MUST 403"
    );
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body["code"], "CSRF_TOKEN_MISSING",
        "错误码必须命中单源常量: {body}"
    );
    assert!(
        body.get("error").is_none(),
        "错误体 MUST 为平铺 {{code,message}}（禁旧式嵌套 error 体）: {body}"
    );
    assert!(body["message"].is_string(), "message 必须存在: {body}");
}

/// ② cookie 鉴权 + 错 CSRF 头 ⇒ 403 `CSRF_TOKEN_MISMATCH`（平铺体）。
#[actix_web::test]
async fn cookie_write_with_wrong_csrf_header_is_403_mismatch() {
    arm_env();
    let pool = connect_test_db().await;
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(enforcer(&pool))
                .route("/csrf-probe", web::post().to(ok_handler)),
        ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/api/csrf-probe")
        .cookie(actix_web::cookie::Cookie::new(
            "access_token",
            issue_token("42", ""),
        ))
        .cookie(actix_web::cookie::Cookie::new(
            "csrf_token",
            "expected-token",
        ))
        .insert_header(("X-CSRF-Token", "wrong-token"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        403,
        "cookie 鉴权写请求错 CSRF 头 MUST 403"
    );
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body["code"], "CSRF_TOKEN_MISMATCH",
        "错误码必须命中单源常量: {body}"
    );
    assert!(
        body.get("error").is_none(),
        "错误体 MUST 为平铺 {{code,message}}: {body}"
    );
}

/// ③ cookie 鉴权 + 正确 double-submit 头 ⇒ 通过（门禁不误伤正向路径）。
#[actix_web::test]
async fn cookie_write_with_matching_csrf_header_passes() {
    arm_env();
    let pool = connect_test_db().await;
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(enforcer(&pool))
                .route("/csrf-probe", web::post().to(ok_handler)),
        ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/api/csrf-probe")
        .cookie(actix_web::cookie::Cookie::new(
            "access_token",
            issue_token("42", ""),
        ))
        .cookie(actix_web::cookie::Cookie::new(
            "csrf_token",
            "matching-token",
        ))
        .insert_header(("X-CSRF-Token", "matching-token"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "cookie + 正确 CSRF 头（且 fail-open）MUST 放行"
    );
}

/// ④ Bearer 鉴权（无 csrf 头）⇒ 通过（Bearer 来源豁免）。
#[actix_web::test]
async fn bearer_write_without_csrf_header_passes() {
    arm_env();
    let pool = connect_test_db().await;
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(enforcer(&pool))
                .route("/csrf-probe", web::post().to(ok_handler)),
        ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/api/csrf-probe")
        .insert_header(("Authorization", format!("Bearer {}", issue_token("42", ""))))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "Bearer 来源 MUST 豁免 CSRF（无 csrf 头也放行）"
    );
}

/// ⑤ 安全方法（GET）⇒ 不校验（无 csrf 头仍放行）。
#[actix_web::test]
async fn safe_method_without_csrf_header_passes() {
    arm_env();
    let pool = connect_test_db().await;
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(enforcer(&pool))
                .route("/csrf-probe", web::get().to(ok_handler)),
        ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/csrf-probe")
        .cookie(actix_web::cookie::Cookie::new(
            "access_token",
            issue_token("42", ""),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "安全方法（GET）MUST 不触发 CSRF 校验"
    );
}

/// ⑥ 次序回归：失效会话写请求 MUST 仍 401（CSRF 门在鉴权成功之后）。
///
/// 本用例同时覆盖两类「先 401 后 CSRF」的次序证据：
///   a) 会话已吊销（sid 在 `isahl_auth.sso_sessions` 不存在）——写请求无 csrf 头；
///   b) 令牌解码失败（无效 token）——写请求无 csrf 头。
#[actix_web::test]
async fn unauthenticated_write_still_401_not_403() {
    arm_env();
    let pool = connect_test_db().await;
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(enforcer(&pool))
                .route("/csrf-probe", web::post().to(ok_handler)),
        ),
    )
    .await;

    // a) 已吊销会话：签名有效、sid 非空但库中不存在（is_session_active=false）
    let req = test::TestRequest::post()
        .uri("/api/csrf-probe")
        .cookie(actix_web::cookie::Cookie::new(
            "access_token",
            issue_token("42", "revoked-session-for-csrf-order-test"),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "失效会话写请求 MUST 仍 401（不得被 CSRF 403 抢先——次序铁律）"
    );

    // b) 无效令牌：解码失败先于 CSRF
    let req = test::TestRequest::post()
        .uri("/api/csrf-probe")
        .cookie(actix_web::cookie::Cookie::new(
            "access_token",
            "not-a-valid-jwt",
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "无效令牌写请求 MUST 仍 401（不得被 CSRF 403 抢先）"
    );
}

/// ⑦ 账号态复查（E2 / 评审候选 2 的 PEP 半）：会话行有效但主体账号被禁用 ⇒ 401。
///
/// 判据（red-before/green-after）：`is_session_active` 增 `auth_users.is_active` join 前，
/// 禁用账号的在期会话仍放行（红）；join 后拒绝（绿）。夹具前缀 `zz-csrf-sess`（用后清）。
#[actix_web::test]
async fn disabled_user_session_is_401() {
    arm_env();
    let pool = connect_test_db().await;
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(enforcer(&pool))
                .route("/csrf-probe", web::get().to(ok_handler)),
        ),
    )
    .await;

    const UID: i64 = 990_201_701;
    const SID: &str = "zz-csrf-sess-disabled-user";

    // 夹具：在期会话 + 账号态切换（幂等预清）
    sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE session_token = $1")
        .bind(SID)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(UID)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, is_active) VALUES ($1, $2, TRUE)",
    )
    .bind(UID)
    .bind("zz-csrf-sess-user")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO isahl_auth.sso_sessions (user_id, session_token, status, expires_at) \
         VALUES ($1, $2, 'active', NOW() + INTERVAL '1 hour')",
    )
    .bind(UID)
    .bind(SID)
    .execute(&pool)
    .await
    .unwrap();

    // ① 账号 active：会话有效 ⇒ 通过会话门（GET 走 PDP fail-open）⇒ 200
    let req = test::TestRequest::get()
        .uri("/api/csrf-probe")
        .cookie(actix_web::cookie::Cookie::new(
            "access_token",
            issue_token("42", SID),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "账号 active 的在期会话 MUST 放行（join 不误伤正常路径）"
    );

    // ② 账号禁用：同一会话 ⇒ 401（账号态 join 生效，fail-closed）
    sqlx::query("UPDATE isahl_auth.auth_users SET is_active = FALSE WHERE id = $1")
        .bind(UID)
        .execute(&pool)
        .await
        .unwrap();
    let req = test::TestRequest::get()
        .uri("/api/csrf-probe")
        .cookie(actix_web::cookie::Cookie::new(
            "access_token",
            issue_token("42", SID),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "禁用账号的在期会话 MUST 401（账号态复查，E2 判据）"
    );

    // 清理（前缀隔离）
    sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE session_token = $1")
        .bind(SID)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(UID)
        .execute(&pool)
        .await
        .unwrap();
}
