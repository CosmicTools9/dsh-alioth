//! Gateway PEP 判定路径测试——list/decide 的发起方式与**合并语义**。
//!
//! 覆盖（无 DB / 无 SSO 依赖：`NgacEnforcer::new_without_pool_with_pdp` 指向本地 stub PDP）：
//!   1. collection read 的 `list` 与 `decide` **并发发起**（stub 记录并发峰值 = 2；
//!      串行实现下峰值为 1——直接观测，而非时序推测）；
//!   2. 两者均放行 → 200，且 `visible_ids` / 列授权注入为下游 header；
//!   3. `decide` 拒绝优先于 `list` 放行 → 403「Permission denied by policies」；
//!   4. `list` 拒绝优先于 `decide` 放行 → 403「List permission check failed or denied」
//!      （错误优先级与串行版逐字一致）；
//!   5. `decide` 不可用（PDP 5xx）→ fail-close 403「Policy decision service unavailable」。

use actix_web::{http::StatusCode, test, web, App, HttpRequest, HttpResponse, HttpServer};
use alioth_gateway::pep::NgacEnforcer;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 与 `NgacEnforcer::new_without_pool*` 内置公钥配对的测试私钥
/// （同 gateway_auth_tdd_test.rs / openapi_pep_integration_test.rs）。
const TEST_SSO_JWT_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm\
-----END PRIVATE KEY-----";

/// 判定路径延迟：让「并发 vs 串行」在 in-flight 计数上可观测。
const DECISION_LATENCY_MS: u64 = 150;

/// stub PDP 行为与观测。
struct StubPdp {
    decide_permitted: bool,
    decide_status: AtomicU16,
    list_permitted: bool,
    list_visible: Option<Vec<i64>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    hits: Mutex<Vec<String>>,
}

impl StubPdp {
    fn new(
        decide_permitted: bool,
        list_permitted: bool,
        list_visible: Option<Vec<i64>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            decide_permitted,
            decide_status: AtomicU16::new(200),
            list_permitted,
            list_visible,
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            hits: Mutex::new(Vec::new()),
        })
    }

    fn hits(&self) -> Vec<String> {
        self.hits.lock().expect("stub hits lock").clone()
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }
}

/// 记录命中 + 并发峰值，并占用一段判定时延。
async fn observe(state: &Arc<StubPdp>, endpoint: &str) {
    state
        .hits
        .lock()
        .expect("stub hits lock")
        .push(endpoint.to_string());
    let in_flight = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    state.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(DECISION_LATENCY_MS)).await;
    state.in_flight.fetch_sub(1, Ordering::SeqCst);
}

async fn decide_handler(state: web::Data<Arc<StubPdp>>) -> HttpResponse {
    observe(&state, "decide").await;
    let status = state.decide_status.load(Ordering::SeqCst);
    if status != 200 {
        return HttpResponse::build(StatusCode::from_u16(status).expect("valid stub status"))
            .finish();
    }
    HttpResponse::Ok().json(json!({ "permitted": state.decide_permitted, "reason": "stub" }))
}

async fn list_handler(state: web::Data<Arc<StubPdp>>) -> HttpResponse {
    observe(&state, "list").await;
    HttpResponse::Ok().json(json!({
        "permitted": state.list_permitted,
        "reason": "stub",
        "visible_ids": state.list_visible,
    }))
}

async fn columns_handler(state: web::Data<Arc<StubPdp>>) -> HttpResponse {
    observe(&state, "columns").await;
    HttpResponse::Ok().json(json!({
        "permitted": true,
        "reason": "stub",
        "columns": ["*"],
    }))
}

/// 启动 stub PDP（独立线程 + 独立 actix System），返回 base url。
fn spawn_stub_pdp(state: Arc<StubPdp>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("stub local addr").to_string();
    drop(listener);

    let bind_addr = addr.clone();
    std::thread::spawn(move || {
        let system = actix_web::rt::System::new();
        system.block_on(async move {
            let _ = HttpServer::new(move || {
                App::new()
                    .app_data(web::Data::new(state.clone()))
                    .route("/api/ngac/pdp/decide", web::post().to(decide_handler))
                    .route("/api/ngac/pdp/list", web::post().to(list_handler))
                    .route("/api/ngac/pdp/columns", web::post().to(columns_handler))
            })
            .workers(1)
            .bind(&bind_addr)
            .expect("bind stub pdp")
            .run()
            .await;
        });
    });

    // 就绪探活（避免首请求竞态）
    for _ in 0..100 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    format!("http://{addr}")
}

/// 签发测试 JWT（ES256，自然人主体、无 sid → 跳过会话吊销校验）。
fn issue_token(sub: &str) -> String {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = json!({
        "sub": sub,
        "exp": now + 3600,
        "iat": now,
        "email": "tester@alioth.test",
        "username": "tester",
        "sid": "",
        "iss": "http://localhost:9002",
        "aud": "http://localhost:9002",
        "scope": "",
        "svc_user_id": 0,
    });
    encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_SSO_JWT_PRIVATE_KEY).expect("test ec key"),
    )
    .expect("sign test token")
}

/// 回显 PEP 注入的授权 header，供断言注入结果。
async fn echo_auth_headers(req: HttpRequest) -> HttpResponse {
    let header = |name: &str| {
        req.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    HttpResponse::Ok().json(json!({
        "visible_ids": header("x-visible-ids"),
        "authorized_columns": header("x-authorized-columns"),
    }))
}

#[tokio::test]
async fn collection_read_runs_list_and_decide_concurrently_and_injects_visible_ids() {
    let stub = StubPdp::new(true, true, Some(vec![7, 8]));
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool_with_pdp(spawn_stub_pdp(
                    Arc::clone(&stub),
                )))
                .route("/engineers", web::get().to(echo_auth_headers)),
        ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/engineers")
        .insert_header(("Authorization", format!("Bearer {}", issue_token("42"))))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body["visible_ids"], "7,8",
        "visible_ids 应注入 X-Visible-Ids"
    );
    assert_eq!(body["authorized_columns"], "*", "read 动作应注入列授权");

    let mut hits = stub.hits();
    hits.sort();
    assert_eq!(
        hits,
        vec!["columns", "decide", "list"],
        "三个阶段都应被调用"
    );
    assert_eq!(
        stub.max_in_flight(),
        2,
        "list 与 decide 必须并发发起（串行实现峰值为 1）"
    );
}

#[tokio::test]
async fn decide_deny_wins_over_permitted_list() {
    let stub = StubPdp::new(false, true, Some(vec![7, 8]));
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool_with_pdp(spawn_stub_pdp(
                    Arc::clone(&stub),
                )))
                .route("/engineers", web::get().to(echo_auth_headers)),
        ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/engineers")
        .insert_header(("Authorization", format!("Bearer {}", issue_token("42"))))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 403);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["reason"], "Permission denied by policies");
}

#[tokio::test]
async fn list_denial_takes_priority_over_permitted_decide() {
    let stub = StubPdp::new(true, false, None);
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool_with_pdp(spawn_stub_pdp(
                    Arc::clone(&stub),
                )))
                .route("/engineers", web::get().to(echo_auth_headers)),
        ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/engineers")
        .insert_header(("Authorization", format!("Bearer {}", issue_token("42"))))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 403);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body["reason"], "List permission check failed or denied",
        "list 失败/拒绝优先返回其 403（错误优先级与串行版一致）"
    );
}

#[tokio::test]
async fn decide_unavailable_fails_closed() {
    let stub = StubPdp::new(true, true, Some(vec![1]));
    stub.decide_status.store(500, Ordering::SeqCst);
    let app = test::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(NgacEnforcer::new_without_pool_with_pdp(spawn_stub_pdp(
                    Arc::clone(&stub),
                )))
                .route("/engineers", web::get().to(echo_auth_headers)),
        ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/engineers")
        .insert_header(("Authorization", format!("Bearer {}", issue_token("42"))))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 403);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["reason"], "Policy decision service unavailable");
}
