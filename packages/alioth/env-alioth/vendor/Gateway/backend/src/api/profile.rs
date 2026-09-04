//! 个人中心 API — 用户个人信息与 NGAC 权限上下文
//!
//! 路由前缀: /api/profile

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use serde::Serialize;

#[derive(Serialize)]
pub struct ProfileResponse {
    pub success: bool,
    pub data: serde_json::Value,
}

/// GET /api/profile/permissions
///
/// 返回当前登录用户在 NGAC 系统中的完整权限上下文，
/// 包括策略类、实体引用和已分配的属性列表。
///
/// 响应格式对齐 ngac::resolve_user_permissions 的输出。
pub async fn get_profile_permissions(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
) -> HttpResponse {
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);

    let user_id = match user_id {
        Some(uid) => uid,
        None => {
            return HttpResponse::Unauthorized().json(serde_json::json!({
                "success": false,
                "error": "未认证用户"
            }));
        }
    };

    match crate::ngac::resolve_user_permissions(pool.get_ref(), user_id).await {
        Ok(data) => HttpResponse::Ok().json(ProfileResponse {
            success: true,
            data,
        }),
        Err(e) => HttpResponse::InternalServerError().json(ProfileResponse {
            success: false,
            data: serde_json::json!({ "error": e }),
        }),
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/profile").route("/permissions", web::get().to(get_profile_permissions)),
    );
}
