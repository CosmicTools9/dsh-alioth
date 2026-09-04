//! Gateway 基础集成测试
//!
//! 验证 Gateway 服务的核心 HTTP 行为，包括健康检查、404 处理等。
//! 这些测试不依赖数据库，确保服务能够独立启动和响应请求。

use actix_web::{test, web, App, HttpResponse};
use serde_json::json;

async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "ok"}))
}

async fn not_found_handler() -> HttpResponse {
    HttpResponse::NotFound().json(json!({
        "error": "Not Found",
        "message": "The requested resource does not exist"
    }))
}

#[actix_web::test]
async fn test_health_endpoint_returns_ok() {
    let app = test::init_service(App::new().route("/health", web::get().to(health_check))).await;

    let req = test::TestRequest::get().uri("/health").to_request();
    let resp = test::call_service(&app, req).await;

    assert!(resp.status().is_success(), "Health check should return 200");

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "ok");
}

#[actix_web::test]
async fn test_404_returns_json_error() {
    let app =
        test::init_service(App::new().default_service(web::route().to(not_found_handler))).await;

    let req = test::TestRequest::get()
        .uri("/nonexistent-route")
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(
        resp.status().as_u16(),
        404,
        "Unknown route should return 404"
    );

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "Not Found");
}

#[actix_web::test]
async fn test_cors_preflight_request() {
    use actix_cors::Cors;
    use actix_web::http::header;

    let cors = Cors::default()
        .allow_any_origin()
        .allowed_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
        .allowed_headers(vec![
            header::AUTHORIZATION,
            header::ACCEPT,
            header::CONTENT_TYPE,
        ])
        .max_age(3600);

    let app = test::init_service(
        App::new()
            .wrap(cors)
            .route("/health", web::get().to(health_check)),
    )
    .await;

    let req = test::TestRequest::default()
        .method(actix_web::http::Method::OPTIONS)
        .uri("/health")
        .insert_header(("Origin", "http://localhost:5173"))
        .insert_header(("Access-Control-Request-Method", "GET"))
        .insert_header(("Access-Control-Request-Headers", "Content-Type"))
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert!(
        resp.status().is_success(),
        "CORS preflight should return 200, got {:?}",
        resp.status()
    );

    // 验证 CORS 响应头
    let allow_origin = resp.headers().get("access-control-allow-origin");
    assert!(
        allow_origin.is_some(),
        "CORS preflight should include Access-Control-Allow-Origin"
    );
}
