//! 通知模块 REST API Handlers
//!
//! 路由前缀: /api/notifications/*

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Result};
use serde_json::json;

use crate::notification::models::{
    CreateSubscriptionRequest, SubscriptionListResponse, UpdateSubscriptionRequest,
};
use crate::notification::repository::SubscriptionRepository;

/// 从请求中提取 user_id（仅受信通道：RequestContext，禁止 X-User-Id header 回退——
/// 入站伪造 header 不可作为身份来源，fix-pep-rls-column-fail-open 4.1）。
fn extract_user_id(req: &HttpRequest) -> Option<i64> {
    req.extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
        .or_else(|| req.extensions().get::<i64>().copied())
}

/// GET /api/notifications/subscriptions
/// 获取当前用户的订阅列表。
pub async fn list_subscriptions(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
) -> Result<HttpResponse> {
    let user_id = match extract_user_id(&req) {
        Some(id) => id,
        None => {
            return Ok(HttpResponse::Unauthorized().json(json!({
                "success": false,
                "error": "Unauthorized"
            })));
        }
    };

    let repo = SubscriptionRepository::from(pool.get_ref().clone());
    let items = match repo.list_by_user(user_id).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": format!("Database error: {}", e)
            })));
        }
    };

    Ok(HttpResponse::Ok().json(json!({
        "success": true,
        "data": SubscriptionListResponse { items }
    })))
}

/// POST /api/notifications/subscriptions
/// 添加订阅。
pub async fn create_subscription(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    body: web::Json<CreateSubscriptionRequest>,
) -> Result<HttpResponse> {
    let user_id = match extract_user_id(&req) {
        Some(id) => id,
        None => {
            return Ok(HttpResponse::Unauthorized().json(json!({
                "success": false,
                "error": "Unauthorized"
            })));
        }
    };

    let repo = SubscriptionRepository::from(pool.get_ref().clone());
    match repo.create(user_id, body.into_inner()).await {
        Ok(sub) => Ok(HttpResponse::Created().json(json!({
            "success": true,
            "data": sub
        }))),
        Err(e) => Ok(HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": format!("Database error: {}", e)
        }))),
    }
}

/// PUT /api/notifications/subscriptions/{id}
/// 更新订阅。
pub async fn update_subscription(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<UpdateSubscriptionRequest>,
) -> Result<HttpResponse> {
    let user_id = match extract_user_id(&req) {
        Some(id) => id,
        None => {
            return Ok(HttpResponse::Unauthorized().json(json!({
                "success": false,
                "error": "Unauthorized"
            })));
        }
    };

    let sub_id = path.into_inner();
    let repo = SubscriptionRepository::from(pool.get_ref().clone());

    // 校验所有权
    match repo.get_by_id(user_id, &sub_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Ok(HttpResponse::Forbidden().json(json!({
                "success": false,
                "error": "Forbidden"
            })));
        }
        Err(e) => {
            return Ok(HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": format!("Database error: {}", e)
            })));
        }
    }

    match repo.update(user_id, &sub_id, body.into_inner()).await {
        Ok(Some(sub)) => Ok(HttpResponse::Ok().json(json!({
            "success": true,
            "data": sub
        }))),
        Ok(None) => Ok(HttpResponse::NotFound().json(json!({
            "success": false,
            "error": "Subscription not found"
        }))),
        Err(e) => Ok(HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": format!("Database error: {}", e)
        }))),
    }
}

/// DELETE /api/notifications/subscriptions/{id}
/// 移除订阅。
pub async fn delete_subscription(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    path: web::Path<String>,
) -> Result<HttpResponse> {
    let user_id = match extract_user_id(&req) {
        Some(id) => id,
        None => {
            return Ok(HttpResponse::Unauthorized().json(json!({
                "success": false,
                "error": "Unauthorized"
            })));
        }
    };

    let sub_id = path.into_inner();
    let repo = SubscriptionRepository::from(pool.get_ref().clone());

    match repo.delete(user_id, &sub_id).await {
        Ok(true) => Ok(HttpResponse::Ok().json(json!({
            "success": true,
            "message": "Subscription removed"
        }))),
        Ok(false) => Ok(HttpResponse::NotFound().json(json!({
            "success": false,
            "error": "Subscription not found"
        }))),
        Err(e) => Ok(HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": format!("Database error: {}", e)
        }))),
    }
}

/// Route 配置。
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/notifications")
            .route("/subscriptions", web::get().to(list_subscriptions))
            .route("/subscriptions", web::post().to(create_subscription))
            .route("/subscriptions/{id}", web::put().to(update_subscription))
            .route("/subscriptions/{id}", web::delete().to(delete_subscription)),
    );
}
