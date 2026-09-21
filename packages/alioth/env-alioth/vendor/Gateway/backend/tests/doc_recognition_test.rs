//! 文档识别服务端点门禁测试（doc-recognition Gateway 接入面）。
//!
//! 覆盖确定性面：`X-Service-Key` fail-closed（无密钥头 → 401；env 是否配置
//! PLATFORM_SERVICE_KEY 不影响 401 断言——两种码同为 Unauthorized）。LLM 路径
//! 由 `doc-recognition` crate 单测 + 运行时组合（DbLlmConfigAdapter 先例）承载，
//! 不在进程内拉起真实 LLM。

use actix_web::{test, web, App};
use sqlx::PgPool;

#[tokio::test]
async fn analyze_rejects_missing_service_key() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    // 惰性连接：密钥校验先于一切池使用，本用例不触库
    let pool = PgPool::connect_lazy(&url).expect("lazy pool");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool))
            .configure(alioth_gateway::api::doc_recognition::configure_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/service/doc-recognition/analyze")
        .set_json(serde_json::json!({ "file_base64": "aGVsbG8=" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "无 X-Service-Key 应 401 fail-closed"
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    let code = body["code"].as_str().unwrap_or_default();
    assert!(
        code == "SERVICE_KEY_UNCONFIGURED" || code == "INVALID_SERVICE_KEY",
        "门禁码应为两者之一，实测 {code}"
    );
}
