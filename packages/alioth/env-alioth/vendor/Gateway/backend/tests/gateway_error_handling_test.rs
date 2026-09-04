//! Gateway 错误处理集成测试
//!
//! 验证 Gateway 返回统一的 JSON 错误响应格式。

use actix_web::{test, web, App, HttpResponse};

/// 模拟一个返回 500 的 handler
async fn internal_error_handler() -> HttpResponse {
    HttpResponse::InternalServerError().json(serde_json::json!({
        "error": "internal_error",
        "message": "Something went wrong"
    }))
}

/// 模拟一个返回 403 的 handler
async fn forbidden_handler() -> HttpResponse {
    HttpResponse::Forbidden().json(serde_json::json!({
        "error": "forbidden",
        "message": "Access denied"
    }))
}

#[actix_web::test]
async fn test_error_response_format() {
    let app = test::init_service(
        App::new()
            .route("/error", web::get().to(internal_error_handler))
            .route("/forbidden", web::get().to(forbidden_handler)),
    )
    .await;

    // 验证 500 错误返回 JSON
    let req = test::TestRequest::get().uri("/error").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 500);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["error"].is_string());
    assert!(body["message"].is_string());

    // 验证 403 错误返回 JSON
    let req = test::TestRequest::get().uri("/forbidden").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 403);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "forbidden");
}
