//! OpenAPI 幂等键（add-openapi-idempotency-keys）集成测试
//!
//! 覆盖（spec delta 三 Requirement 全部场景）：
//!   1. IDEM-001 首次执行 + 存储快照，同 key 同指纹重放 → 快照 + Replayed，handler 不重复执行
//!   2. IDEM-002 同 key 异 payload → 409 IDEMPOTENCY_PAYLOAD_MISMATCH
//!   3. IDEM-003 in_progress 并发 → 409 IDEMPOTENCY_REQUEST_IN_PROGRESS + Retry-After
//!   4. IDEM-004 自然人令牌带 key → 透传（handler 执行，不建幂等记录）
//!   5. IDEM-005 GET 带 key → 透传
//!   6. IDEM-006 无 key header → 透传
//!   7. IDEM-007 首次 5xx → 记录删除，同 key 重试重新执行
//!   8. IDEM-008 X-Api-Version 维度隔离（同 key 跨版本互不冲突）
//!   9. IDEM-009 非法 X-Api-Version → 400
//!
//! 依赖真实测试库（aliothstudio_test，含迁移 033 的表）。测试数据自建自清。

use actix_web::{test, web, App, HttpResponse};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ::common::testing::connect_test_db;

/// 测试用 EC P-256 私钥（与 NgacEnforcer::new_without_pool 内置公钥配对）。
const TEST_SSO_JWT_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs
\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0
\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm
\
-----END PRIVATE KEY-----";

/// 签发测试 JWT（ES256，含 svc_user_id / scope）。
fn issue_token(sub: &str, svc_user_id: i64, scope: &str) -> String {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = serde_json::json!({
        "sub": sub,
        "exp": now + 3600,
        "iat": now,
        "email": "",
        "username": "",
        "sid": "",
        "iss": "http://localhost:9002",
        "aud": "http://localhost:9002",
        "scope": scope,
        "svc_user_id": svc_user_id,
    });
    let header = Header::new(Algorithm::ES256);
    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_SSO_JWT_PRIVATE_KEY).unwrap(),
    )
    .unwrap()
}

/// 创建测试 client + 服务用户 + 订阅（free），返回 (client_id, svc_user_id)。
async fn create_test_client(pool: &PgPool, suffix: &str) -> (String, i64) {
    let client_id = format!("idem_test_{}", suffix);
    let svc_user = sqlx::query_scalar::<_, i64>(
        "INSERT INTO isahl_auth.auth_users \
         (name, username, email, password_hash, user_type, is_active, status, \
          created_at, updated_at, failed_login_attempts, notification_preferences) \
         VALUES ($1, $2, NULL, NULL, 'service', TRUE, 'active', NOW(), NOW(), 0, '{}'::jsonb) \
         RETURNING id",
    )
    .bind(format!("svc-{}", client_id))
    .bind(format!("svc:{}", client_id))
    .fetch_one(pool)
    .await
    .expect("create service user");

    let client_row: (i64,) = sqlx::query_as(
        "INSERT INTO isahl_auth.api_clients \
         (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled) \
         VALUES ($1, 'oauth2', 'idem-test', '', $2::TEXT[], $3, TRUE) \
         RETURNING id",
    )
    .bind(&client_id)
    .bind(&["read:units".to_string()])
    .bind(svc_user)
    .fetch_one(pool)
    .await
    .expect("create client");

    let plan_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.api_plans WHERE code = 'free'")
            .fetch_one(pool)
            .await
            .expect("find free plan");

    sqlx::query(
        "INSERT INTO isahl_auth.api_subscriptions (fk_client, fk_plan, status) \
                 VALUES ($1, $2, 'active')",
    )
    .bind(client_row.0)
    .bind(plan_id)
    .execute(pool)
    .await
    .expect("create subscription");

    (client_id, svc_user)
}

async fn cleanup_test_client(pool: &PgPool, client_id: &str) {
    // 幂等记录（FK 依赖 api_clients，必须先清）
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.api_idempotency_keys k
           USING isahl_auth.api_clients c
           WHERE k.fk_client = c.id AND c.client_id = $1"#,
    )
    .bind(client_id)
    .execute(pool)
    .await;
    // 订阅 / client / 服务用户
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.api_subscriptions s
           USING isahl_auth.api_clients c
           WHERE s.fk_client = c.id AND c.client_id = $1"#,
    )
    .bind(client_id)
    .execute(pool)
    .await;
    let svc: Option<i64> = sqlx::query_scalar(
        "SELECT fk_service_user FROM isahl_auth.api_clients WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    if let Some(uid) = svc {
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .execute(pool)
            .await;
    }
    let _ = sqlx::query("DELETE FROM isahl_auth.api_clients WHERE client_id = $1")
        .bind(client_id)
        .execute(pool)
        .await;
}

/// 构造带幂等中间件的测试 app：POST /api/service/measurement/units
/// handler 执行计数（证明重放不重复执行）+ 可控 5xx（IDEM-007）。
macro_rules! idem_app {
    ($pool:expr, $count:expr, $fail_once:expr) => {{
        let pool = $pool;
        let count = $count;
        let fail_once = $fail_once;
        test::init_service(
            App::new()
                .wrap(alioth_gateway::openapi::idempotency::IdempotencyMiddleware::new(pool))
                .route("/api/service/measurement/units", web::post().to(move || {
                    let count = count.clone();
                    let fail_once = fail_once.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        // 先 load 再 fetch_sub：0 值 fetch_sub 回绕为 MAX，避免误读
                        if fail_once.load(Ordering::SeqCst) > 0 {
                            fail_once.fetch_sub(1, Ordering::SeqCst);
                            HttpResponse::InternalServerError().json(json!({"error": "boom"}))
                        } else {
                            HttpResponse::Ok().json(json!({"data": {"id": 42}}))
                        }
                    }
                })),
        )
        .await
    }};
}

/// 读取幂等记录数。
async fn idempotency_row_count(pool: &PgPool, client_id: &str) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl_auth.api_idempotency_keys k
           JOIN isahl_auth.api_clients c ON k.fk_client = c.id
           WHERE c.client_id = $1"#,
    )
    .bind(client_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

// ============================================================================
// IDEM-001: 首次执行存储 + 重放快照（不重复执行 handler）
// ============================================================================
#[tokio::test]
async fn idem_001_replay_returns_snapshot_without_re_execution() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, svc_user) = create_test_client(&pool, &suffix).await;

    let count = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(AtomicUsize::new(0));
    let app = idem_app!(pool.clone(), count.clone(), fail_once);
    let token = issue_token(&format!("client:{}", client_id), svc_user, "read:units");

    // 首次：handler 执行 1 次
    let req = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-001"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(
        resp.headers().get("Idempotency-Replayed").is_none(),
        "首次响应不应带 Replayed 头"
    );

    // 重放：同 key 同 body → 快照 + Replayed，handler 不重复执行
    let req2 = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-001"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_eq!(resp2.status().as_u16(), 200);
    assert_eq!(count.load(Ordering::SeqCst), 1, "重放不得重复执行 handler");
    assert_eq!(
        resp2
            .headers()
            .get("Idempotency-Replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );

    // 快照 body 与首次一致
    let body: serde_json::Value = test::read_body_json(resp2).await;
    assert_eq!(body, json!({"data": {"id": 42}}));

    // 记录已入库（completed）
    let rows = idempotency_row_count(&pool, &client_id).await;
    assert_eq!(rows, 1);

    cleanup_test_client(&pool, &client_id).await;
}

// ============================================================================
// IDEM-002: 同 key 异 payload → 409
// ============================================================================
#[tokio::test]
async fn idem_002_same_key_different_payload_conflict() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, svc_user) = create_test_client(&pool, &suffix).await;

    let count = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(AtomicUsize::new(0));
    let app = idem_app!(pool.clone(), count.clone(), fail_once);
    let token = issue_token(&format!("client:{}", client_id), svc_user, "read:units");

    // 首次（key=k-002, body A）
    let req = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-002"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);

    // 同 key 不同 body → 409
    let req2 = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-002"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "gadget"})).unwrap())
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_eq!(resp2.status().as_u16(), 409);
    let body: serde_json::Value = test::read_body_json(resp2).await;
    assert_eq!(body["error"], "IDEMPOTENCY_PAYLOAD_MISMATCH");
    assert_eq!(count.load(Ordering::SeqCst), 1, "冲突请求不得执行 handler");

    cleanup_test_client(&pool, &client_id).await;
}

// ============================================================================
// IDEM-003: in_progress 并发 → 409 + Retry-After
// ============================================================================
#[tokio::test]
async fn idem_003_in_progress_conflict_retry_after() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, svc_user) = create_test_client(&pool, &suffix).await;

    let count = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(AtomicUsize::new(0));
    let app = idem_app!(pool.clone(), count.clone(), fail_once);
    let token = issue_token(&format!("client:{}", client_id), svc_user, "read:units");

    // 预置 in_progress 记录（模拟 leader 仍在执行）——指纹须与请求一致，
    // 否则先命中 PAYLOAD_MISMATCH 而非 IN_PROGRESS
    let fk: i64 = sqlx::query_scalar("SELECT id FROM isahl_auth.api_clients WHERE client_id = $1")
        .bind(&client_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let body = serde_json::to_vec(&json!({"name": "widget"})).unwrap();
    let mut hasher = Sha256::new();
    hasher.update("POST".as_bytes());
    hasher.update("/api/service/measurement/units".as_bytes());
    hasher.update(&body);
    let fingerprint: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    sqlx::query(
        r#"INSERT INTO isahl_auth.api_idempotency_keys
           (id, fk_client, api_version, idem_key, method, path, request_fingerprint, state)
           VALUES (isahl.gen_next_zuid(), $1, 'v1', 'k-003', 'POST', '/api/service/measurement/units', $2, 'in_progress')"#,
    )
    .bind(fk)
    .bind(fingerprint)
    .execute(&pool)
    .await
    .expect("insert in_progress row");

    let req = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-003"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 409);
    assert_eq!(
        resp.headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "IDEMPOTENCY_REQUEST_IN_PROGRESS");
    assert_eq!(count.load(Ordering::SeqCst), 0);

    cleanup_test_client(&pool, &client_id).await;
}

// ============================================================================
// IDEM-004: 自然人令牌带 key → 透传（handler 执行，不建记录）
// ============================================================================
#[tokio::test]
async fn idem_004_natural_user_passthrough() {
    let pool = connect_test_db().await;
    let count = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(AtomicUsize::new(0));
    let app = idem_app!(pool.clone(), count.clone(), fail_once);
    let token = issue_token("1002", 0, ""); // svc_user_id=0

    let req = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-nat"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(count.load(Ordering::SeqCst), 1, "自然人应正常执行 handler");
    assert!(resp.headers().get("Idempotency-Replayed").is_none());

    // 自然人透传不得写幂等记录：全局计数在请求前后不变
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM isahl_auth.api_idempotency_keys")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
    let req2 = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-nat-2"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let _ = test::call_service(&app, req2).await;
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM isahl_auth.api_idempotency_keys")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
    assert_eq!(before, after, "自然人透传不得写幂等记录");
}

// ============================================================================
// IDEM-005: GET 带 key → 透传
// ============================================================================
#[tokio::test]
async fn idem_005_get_passthrough() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, svc_user) = create_test_client(&pool, &suffix).await;

    // GET 路由单独构造（build_app 只注册 POST）
    let app = test::init_service(
        App::new()
            .wrap(alioth_gateway::openapi::idempotency::IdempotencyMiddleware::new(pool.clone()))
            .route(
                "/api/service/measurement/units",
                web::get().to(|| async { HttpResponse::Ok().json(json!({"data": []})) }),
            ),
    )
    .await;
    let token = issue_token(&format!("client:{}", client_id), svc_user, "read:units");

    let req = test::TestRequest::get()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-get"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    assert!(resp.headers().get("Idempotency-Replayed").is_none());
    assert_eq!(idempotency_row_count(&pool, &client_id).await, 0);

    cleanup_test_client(&pool, &client_id).await;
}

// ============================================================================
// IDEM-006: 无 key header → 透传
// ============================================================================
#[tokio::test]
async fn idem_006_no_key_passthrough() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, svc_user) = create_test_client(&pool, &suffix).await;

    let count = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(AtomicUsize::new(0));
    let app = idem_app!(pool.clone(), count.clone(), fail_once);
    let token = issue_token(&format!("client:{}", client_id), svc_user, "read:units");

    let req = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(idempotency_row_count(&pool, &client_id).await, 0);

    cleanup_test_client(&pool, &client_id).await;
}

// ============================================================================
// IDEM-007: 首次 5xx → 记录删除，同 key 重试重新执行
// ============================================================================
#[tokio::test]
async fn idem_007_server_error_frees_idempotency_slot() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, svc_user) = create_test_client(&pool, &suffix).await;

    let count = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(AtomicUsize::new(1)); // 仅首次 500
    let app = idem_app!(pool.clone(), count.clone(), fail_once);
    let token = issue_token(&format!("client:{}", client_id), svc_user, "read:units");

    // 首次 → 500
    let req = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-007"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 500);

    // 5xx 不占用幂等槽
    assert_eq!(idempotency_row_count(&pool, &client_id).await, 0);

    // 同 key 重试 → 重新执行（200）
    let req2 = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-007"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_eq!(resp2.status().as_u16(), 200);
    assert_eq!(count.load(Ordering::SeqCst), 2, "5xx 后同 key 应重新执行");
    assert!(resp2.headers().get("Idempotency-Replayed").is_none());

    cleanup_test_client(&pool, &client_id).await;
}

// ============================================================================
// IDEM-008: X-Api-Version 维度隔离（同 key 跨版本互不冲突）
// ============================================================================
#[tokio::test]
async fn idem_008_api_version_isolation() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, svc_user) = create_test_client(&pool, &suffix).await;

    let count = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(AtomicUsize::new(0));
    let app = idem_app!(pool.clone(), count.clone(), fail_once);
    let token = issue_token(&format!("client:{}", client_id), svc_user, "read:units");

    // v1
    let req = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-008"))
        .insert_header(("X-Api-Version", "v1"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);

    // v2 同 key → 独立执行（不冲突、不重放）
    let req2 = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-008"))
        .insert_header(("X-Api-Version", "v2"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_eq!(resp2.status().as_u16(), 200);
    assert!(resp2.headers().get("Idempotency-Replayed").is_none());
    assert_eq!(count.load(Ordering::SeqCst), 2, "跨版本同 key 应各自执行");

    // v1 重放仍命中 v1 快照
    let req3 = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-008"))
        .insert_header(("X-Api-Version", "v1"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp3 = test::call_service(&app, req3).await;
    assert_eq!(resp3.status().as_u16(), 200);
    assert_eq!(
        resp3
            .headers()
            .get("Idempotency-Replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );
    assert_eq!(count.load(Ordering::SeqCst), 2, "v1 重放不得再执行");

    assert_eq!(idempotency_row_count(&pool, &client_id).await, 2);

    cleanup_test_client(&pool, &client_id).await;
}

// ============================================================================
// IDEM-009: 非法 X-Api-Version → 400
// ============================================================================
#[tokio::test]
async fn idem_009_invalid_api_version_rejected() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, svc_user) = create_test_client(&pool, &suffix).await;

    let count = Arc::new(AtomicUsize::new(0));
    let fail_once = Arc::new(AtomicUsize::new(0));
    let app = idem_app!(pool.clone(), count.clone(), fail_once);
    let token = issue_token(&format!("client:{}", client_id), svc_user, "read:units");

    let req = test::TestRequest::post()
        .uri("/api/service/measurement/units")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .insert_header(("Idempotency-Key", "k-009"))
        .insert_header(("X-Api-Version", "-bad"))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(serde_json::to_vec(&json!({"name": "widget"})).unwrap())
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 400);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "INVALID_API_VERSION");
    assert_eq!(count.load(Ordering::SeqCst), 0);

    cleanup_test_client(&pool, &client_id).await;
}
