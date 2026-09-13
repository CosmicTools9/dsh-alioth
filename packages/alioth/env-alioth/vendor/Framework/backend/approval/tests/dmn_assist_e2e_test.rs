//! dmn-assist 端点端到端可达性（migrate-dmn-assist-to-framework）：
//! - 路由经 approval::configure_routes 注册（ns 服务化部署 delegate 同一入口）→
//!   请求可达（400/503 而非 404）
//! - 空 message → 400 明细；LLM 未配置 → 503 fail-closed（不 500 不 404）
//! - 真实 LLM 端到端（NL → 整表）依赖环境 LLM 配置（zc_id_prot-llm_config /
//!   LLM_API_KEY env），此处锚路由与 fail-closed 行为。

use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval;
use serde_json::{json, Value};
mod common;
use common::setup_test_schema;

const USER_ID: i64 = 425401;

macro_rules! build_app {
    ($pool:expr) => {{
        let ctx = ::common::context::RequestContext::with_username(
            USER_ID,
            "dmn-assist@test.local",
            "dmn-assist-test",
        );
        test::init_service(
            App::new().app_data(web::Data::new($pool.clone())).service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    // 与 ns 服务化部署同一入口（commitment delegate
                    // approval::configure_routes）——dmn-assist 注册于此
                    .configure(approval::configure_routes),
            ),
        )
        .await
    }};
}

macro_rules! post_json {
    ($app:expr, $uri:expr, $body:expr) => {{
        let resp = test::call_service(
            $app,
            test::TestRequest::post()
                .uri($uri)
                .set_json($body)
                .to_request(),
        )
        .await;
        let status: u16 = resp.status().as_u16();
        let body: Value = test::read_body_json(resp).await;
        (status, body)
    }};
}

#[tokio::test]
async fn dmn_assist_empty_message_returns_400() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = build_app!(&pool);

    // 空 message：路由可达 + 参数校验 400（fail-closed 明细，非 404）
    let (s, b) = post_json!(
        &app,
        "/test/approval-flows/dmn-assist",
        json!({"message": "", "context_fields": [], "outputs": [], "allow_unlisted": false})
    );
    assert_eq!(s, 400, "空 message 应 400: {b}");
    assert!(b.to_string().contains("message"), "错误应指 message: {b}");
}

#[tokio::test]
async fn dmn_fix_without_existing_returns_400() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        "/test/approval-flows/dmn-fix",
        json!({"message": "改为兜底", "context_fields": [], "outputs": [], "allow_unlisted": false})
    );
    assert_eq!(s, 400, "dmn-fix 无 existing 应 400: {b}");
    assert!(b.to_string().contains("existing"), "错误应指 existing: {b}");
}

#[tokio::test]
async fn dmn_assist_without_llm_config_fail_closed_503() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = build_app!(&pool);

    // 无 LLM 配置（测试环境 DB 空 + env 无 LLM_API_KEY）→ 503 fail-closed，
    // 端点必须可达（非 404——证明路由经 approval configure_routes 注册）
    let (s, b) = post_json!(
        &app,
        "/test/approval-flows/dmn-assist",
        json!({
            "message": "金额≥10000 走大额审批，其余走普通审批",
            "context_fields": ["amount"],
            "outputs": ["go-big", "go-normal"],
            "allow_unlisted": false
        })
    );
    assert_eq!(s, 503, "无 LLM 配置应 503 fail-closed: {b}");
    assert!(
        b.to_string().contains("LLM") || b.to_string().contains("不可用"),
        "错误应指 LLM 不可用: {b}"
    );
}
