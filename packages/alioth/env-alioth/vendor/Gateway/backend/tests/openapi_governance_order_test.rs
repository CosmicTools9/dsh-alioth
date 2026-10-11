//! OpenAPI 请求治理装配序回归（change `fix-openapi-review-findings` §1；规约
//! `openapi-external-access :: governance-execution-order-and-accounting`）。
//!
//! 三组：
//! 1. `registration_order_semantics_is_lifo`（无 DB）：标记中间件钉住 actix-web
//!    「后注册者先执行」语义（治理装配的判据基石；防依赖升级漂移）。
//! 2. `forged_token_leaves_no_side_effects`（DB）：伪造签名服务令牌 → PEP 401 且
//!    `api_usage` 零行 / `api_idempotency_keys` 零行 / 限流桶零消耗（随后合法请求仍 200）。
//! 3. `replayed_request_is_metered`（DB）：幂等重放返回快照且 `api_usage` 为该重放 +1。
//!
//! 装配面 = `OpenApiGovernance::install`（与组合根同一函数）——生产/测试同源；
//! 需要授权透传的用例走 `NGAC_FAIL_OPEN=true`（dev/standalone 交付语义，与 pep 套件同法）。
//!
//! 依赖真实测试库（aliothstudio_test）。测试数据自建自清。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use actix_web::body::EitherBody;
use actix_web::dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{test, web, App, Error, HttpResponse};
use futures::future::{ok, LocalBoxFuture, Ready};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use sqlx::PgPool;

use ::common::testing::connect_test_db;
use alioth_gateway::openapi::governance::OpenApiGovernance;
use alioth_gateway::pep::NgacEnforcer;

/// 测试 EC P-256 密钥对（本文件专属 fixtures；非生产/非其它测试密钥）。
const TEST_PRIVATE_KEY: &[u8] = include_bytes!("fixtures/openapi_governance_ec_private.pem");
const TEST_PUBLIC_KEY: &[u8] = include_bytes!("fixtures/openapi_governance_ec_public.pem");

const TEST_ISSUER: &str = "http://localhost:9002";

// ============================================================================
// 组 1: actix 注册序语义（无 DB）
// ============================================================================

/// 标记中间件：请求经过时记录自己的到达序号（无锁：共享自增序号 + 各自观察位）。
#[derive(Clone)]
struct Marker {
    seq: Arc<AtomicUsize>,
    seen: Arc<AtomicUsize>,
}

impl<S, B> Transform<S, ServiceRequest> for Marker
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = MarkerService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(MarkerService {
            seq: self.seq.clone(),
            seen: self.seen.clone(),
            service,
        })
    }
}

struct MarkerService<S> {
    seq: Arc<AtomicUsize>,
    seen: Arc<AtomicUsize>,
    service: S,
}

impl<S, B> Service<ServiceRequest> for MarkerService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let pos = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.seen.store(pos, Ordering::SeqCst);
        let fut = self.service.call(req);
        Box::pin(async move { fut.await.map(|r| r.map_into_left_body()) })
    }
}

/// actix-web 4 语义锚：`wrap(A).wrap(B)` ⇒ 请求先经 B 后经 A（**后注册者先执行**）。
/// 治理装配（`OpenApiGovernance::install`）的注册序即按该语义书写；该测试变红即语义漂移。
#[tokio::test]
async fn registration_order_semantics_is_lifo() {
    let seq = Arc::new(AtomicUsize::new(0));
    let a_seen = Arc::new(AtomicUsize::new(0));
    let b_seen = Arc::new(AtomicUsize::new(0));
    let app = test::init_service(
        App::new()
            .wrap(Marker {
                seq: seq.clone(),
                seen: a_seen.clone(),
            })
            .wrap(Marker {
                seq: seq.clone(),
                seen: b_seen.clone(),
            })
            .route(
                "/probe",
                web::get().to(|| async { HttpResponse::Ok().finish() }),
            ),
    )
    .await;

    let resp = test::call_service(&app, test::TestRequest::get().uri("/probe").to_request()).await;
    assert!(resp.status().is_success());
    let (a, b) = (a_seen.load(Ordering::SeqCst), b_seen.load(Ordering::SeqCst));
    assert!(a > 0 && b > 0, "两侧标记中间件都 MUST 执行（a={a} b={b}）");
    assert!(
        b < a,
        "actix-web 语义：后注册者先执行（B 在 A 前；a={a} b={b}）"
    );
}

// ============================================================================
// 组 2/3: 治理组合（同走 install；真实 PEP + DB 夹具）
// ============================================================================

/// 签发测试 JWT（ES256，含 svc_user_id / scope；iss/aud 为绑定值）。
fn issue_token(sub: &str, svc_user_id: i64, scope: &str) -> String {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = serde_json::json!({
        "sub": sub,
        "exp": now + 3600,
        "iat": now,
        "email": "",
        "username": "",
        "sid": "",
        "iss": TEST_ISSUER,
        "aud": TEST_ISSUER,
        "scope": scope,
        "svc_user_id": svc_user_id,
    });
    let header = Header::new(Algorithm::ES256);
    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY).unwrap(),
    )
    .unwrap()
}

/// 结构合法但签名段被替换的令牌（伪造面）。
fn forged(token: &str) -> String {
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT 三段结构");
    format!("{}.{}.Zm9yZ2VkLXNpZ25hdHVyZQ", parts[0], parts[1])
}

/// 治理管线用 PEP：测试公钥 + 无 PDP 端点（配合 NGAC_FAIL_OPEN 透传）+ iss/aud 绑定。
fn test_pep(pool: &PgPool) -> NgacEnforcer {
    NgacEnforcer::new(
        pool.clone(),
        TEST_PUBLIC_KEY.to_vec(),
        Vec::new(),
        String::new(),
    )
    .with_token_binding(TEST_ISSUER.to_string(), TEST_ISSUER.to_string())
}

/// 建夹具：服务用户 + client + 专属 plan（rps=1 / burst=2 / 配额不限）+ 订阅。
/// 返回 (client_id, subscription_id, svc_user_id)。
///
/// **自清理前置**：同后缀残留（前次 panic 未 cleanup）先清再建——复跑幂等。
async fn create_fixture(pool: &PgPool, suffix: &str) -> (String, i64, i64) {
    let client_id = format!("gov_order_test_{suffix}");
    cleanup_fixture(pool, &client_id).await;

    let svc_user: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users \
         (username, password_hash, user_type, is_active, status, \
          created_at, updated_at, failed_login_attempts, notification_preferences) \
         VALUES ($1, NULL, 'service', TRUE, 'active', NOW(), NOW(), 0, '{}'::jsonb) \
         RETURNING id",
    )
    .bind(format!("svc:{}", client_id))
    .fetch_one(pool)
    .await
    .expect("create service user");

    let client_row: (i64,) = sqlx::query_as(
        "INSERT INTO isahl_auth.api_clients \
         (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled) \
         VALUES ($1, 'oauth2', 'gov-order-test', '', $2::TEXT[], $3, TRUE) \
         RETURNING id",
    )
    .bind(&client_id)
    .bind(&["*".to_string()])
    .bind(svc_user)
    .fetch_one(pool)
    .await
    .expect("create client");

    let plan_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.api_plans \
         (id, code, tier, rate_limit_rps, burst, quota_daily, quota_monthly, \
          sla_availability, sla_p95_ms, support_level, enabled) \
         VALUES (isahl.gen_next_zuid(), $1, 9, 1.0, 2, 0, 0, 0.990, 0, 'community', TRUE) \
         RETURNING id",
    )
    .bind(format!("gov_order_test_plan_{suffix}"))
    .fetch_one(pool)
    .await
    .expect("create plan");

    let sub_row: (i64,) = sqlx::query_as(
        "INSERT INTO isahl_auth.api_subscriptions (fk_client, fk_plan, status) \
         VALUES ($1, $2, 'active') RETURNING id",
    )
    .bind(client_row.0)
    .bind(plan_id)
    .fetch_one(pool)
    .await
    .expect("create subscription");

    (client_id, sub_row.0, svc_user)
}

async fn cleanup_fixture(pool: &PgPool, client_id: &str) {
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.api_idempotency_keys k
           USING isahl_auth.api_clients c
           WHERE k.fk_client = c.id AND c.client_id = $1"#,
    )
    .bind(client_id)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.api_usage u
           USING isahl_auth.api_subscriptions s, isahl_auth.api_clients c
           WHERE u.fk_subscription = s.id AND s.fk_client = c.id AND c.client_id = $1"#,
    )
    .bind(client_id)
    .execute(pool)
    .await;
    let plan_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT s.fk_plan FROM isahl_auth.api_subscriptions s
           JOIN isahl_auth.api_clients c ON s.fk_client = c.id
           WHERE c.client_id = $1 LIMIT 1"#,
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.api_subscriptions s
           USING isahl_auth.api_clients c
           WHERE s.fk_client = c.id AND c.client_id = $1"#,
    )
    .bind(client_id)
    .execute(pool)
    .await;
    if let Some(pid) = plan_id {
        let _ = sqlx::query("DELETE FROM isahl_auth.api_plans WHERE id = $1")
            .bind(pid)
            .execute(pool)
            .await;
    }
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

async fn usage_count(pool: &PgPool, client_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.api_usage u \
         JOIN isahl_auth.api_subscriptions s ON u.fk_subscription = s.id \
         JOIN isahl_auth.api_clients c ON s.fk_client = c.id \
         WHERE c.client_id = $1",
    )
    .bind(client_id)
    .fetch_one(pool)
    .await
    .unwrap_or(-1)
}

async fn idempotency_count(pool: &PgPool, client_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.api_idempotency_keys k \
         JOIN isahl_auth.api_clients c ON k.fk_client = c.id \
         WHERE c.client_id = $1",
    )
    .bind(client_id)
    .fetch_one(pool)
    .await
    .unwrap_or(-1)
}

/// 轮询等待用量行数到达期望值（计量写入为异步批量，worker 100ms 收集窗）。
async fn wait_usage_count(pool: &PgPool, client_id: &str, expected: i64, timeout: Duration) -> i64 {
    let deadline = Instant::now() + timeout;
    loop {
        let n = usage_count(pool, client_id).await;
        if n == expected || Instant::now() >= deadline {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 计数 handler 调用次数并回 200。
fn counting_handler(
    calls: Arc<AtomicUsize>,
) -> impl Fn() -> futures::future::Ready<HttpResponse> + Clone + 'static {
    move || {
        calls.fetch_add(1, Ordering::SeqCst);
        futures::future::ready(HttpResponse::Ok().json(serde_json::json!({"data": []})))
    }
}

/// 伪造签名令牌不产生任何计量/幂等/限流副作用（治理序 = PEP 先于 metering/idempotency）。
///
/// 红（旧序 idempotency → metering → … → PEP）：伪造请求会写 `api_usage`（401 行）与
/// `api_idempotency_keys`、耗尽限流桶 ⇒ 后续合法请求 429。
/// 绿（目标序）：PEP 先行 401 短路，全部副作用为零，合法请求不受影响。
#[tokio::test]
async fn forged_token_leaves_no_side_effects() {
    std::env::set_var("NGAC_FAIL_OPEN", "true");
    let pool = connect_test_db().await;
    let (client_id, _sub, svc_user) = create_fixture(&pool, "forged").await;

    let calls = Arc::new(AtomicUsize::new(0));
    let governance = OpenApiGovernance::new(pool.clone());
    let app = test::init_service(
        App::new().service(governance.install(test_pep(&pool)).route(
            "/service/measurement/units",
            web::post().to(counting_handler(calls.clone())),
        )),
    )
    .await;

    // 合法令牌 → 伪造签名段
    let valid = issue_token(&format!("client:{}", client_id), svc_user, "*");
    let bad = forged(&valid);

    // 连发 3 个伪造请求（各自独立幂等键，覆盖幂等面写入路径）
    for i in 0..3 {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/service/measurement/units")
                .insert_header(("Authorization", format!("Bearer {}", bad)))
                .insert_header(("Idempotency-Key", format!("forged-{}", i)))
                .set_json(serde_json::json!({"x": 1}))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status().as_u16(), 401, "伪造签名 MUST 由 PEP 401 短路");
    }

    // 异步计量（若有）应有 100ms 收集窗；给足窗口后断言零副作用
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        usage_count(&pool, &client_id).await,
        0,
        "未通过鉴权的请求 MUST NOT 计量"
    );
    assert_eq!(
        idempotency_count(&pool, &client_id).await,
        0,
        "未通过鉴权的请求 MUST NOT 写/占用幂等键"
    );

    // 限流桶未被伪造请求消耗：合法请求必须成功（burst=2；旧序下 3 个伪造请求已耗尽桶）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/service/measurement/units")
            .insert_header(("Authorization", format!("Bearer {}", valid)))
            .set_json(serde_json::json!({"x": 1}))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "伪造请求 MUST NOT 消耗限流桶（合法请求不受影响）"
    );
    assert!(calls.load(Ordering::SeqCst) >= 1, "合法请求应到达 handler");

    // 合法请求被计量（顺带确认计量链路本身工作）
    assert_eq!(
        wait_usage_count(&pool, &client_id, 1, Duration::from_secs(3)).await,
        1,
        "合法请求 MUST 计量"
    );

    cleanup_fixture(&pool, &client_id).await;
}

/// 幂等重放仍计入用量与配额（metering 先于 idempotency）。
///
/// 红（旧序）：重放由 idempotency 最外层短路，不经 metering ⇒ `api_usage` 不增加。
/// 绿（目标序）：重放先过 metering ⇒ 用量 +1。
#[tokio::test]
async fn replayed_request_is_metered() {
    std::env::set_var("NGAC_FAIL_OPEN", "true");
    let pool = connect_test_db().await;
    let (client_id, _sub, svc_user) = create_fixture(&pool, "replay").await;

    let calls = Arc::new(AtomicUsize::new(0));
    let governance = OpenApiGovernance::new(pool.clone());
    let app = test::init_service(
        App::new().service(governance.install(test_pep(&pool)).route(
            "/service/measurement/units",
            web::post().to(counting_handler(calls.clone())),
        )),
    )
    .await;

    let token = issue_token(&format!("client:{}", client_id), svc_user, "*");
    let body = serde_json::json!({"y": 2});

    // 首次：leader 执行 handler 并存储快照
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/service/measurement/units")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .insert_header(("Idempotency-Key", "replay-k1"))
            .set_json(body.clone())
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        wait_usage_count(&pool, &client_id, 1, Duration::from_secs(3)).await,
        1,
        "首次请求 MUST 计量"
    );

    // 重放：同 key 同体 → 快照返回 + 仍计量
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/service/measurement/units")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .insert_header(("Idempotency-Key", "replay-k1"))
            .set_json(body)
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers()
            .get("Idempotency-Replayed")
            .map(|v| v.to_str().unwrap()),
        Some("true"),
        "重放 MUST 返回快照标记"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "重放 MUST NOT 再次执行 handler"
    );
    assert_eq!(
        wait_usage_count(&pool, &client_id, 2, Duration::from_secs(3)).await,
        2,
        "重放请求 MUST 经 metering 计量（用量 +1）"
    );

    cleanup_fixture(&pool, &client_id).await;
}
