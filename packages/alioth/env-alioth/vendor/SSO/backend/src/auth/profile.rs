//! 用户个人信息管理 handlers
//!
//! 已登录用户查看和更新自己的个人信息。
//!
//! PATCH /auth/me/profile — 更新个人资料
use sqlx::AssertSqlSafe;

use actix_web::{web, HttpRequest, HttpResponse};
use serde::Deserialize;

use super::jwt::{decode_token_any, Claims};
use super::login::AuthError;

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub display_name: Option<String>,
}

/// PATCH /auth/me/profile
///
/// 更新当前登录用户的基本信息（name, display_name）。
pub async fn update_profile(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<super::AuthState>,
    body: web::Json<UpdateProfileRequest>,
) -> HttpResponse {
    // 令牌提取走共享实现（scoped cookie → Bearer → 环境 cookie；见 jwt::extract_token）
    let access_token = match crate::auth::jwt::extract_token(&req) {
        Some(t) => t,
        None => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "No authentication token".to_string(),
            })
        }
    };

    let claims: Claims = match decode_token_any(&access_token, &state.verification_keys()) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Token decode error in profile update: {}", e);
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or expired token".to_string(),
            });
        }
    };

    // CSRF 自检（design.md §4「自检面」）：公开前缀下 cookie 鉴权端点，鉴权成功后校验
    if let Some(resp) = crate::auth::csrf_self_check(&req) {
        return resp;
    }

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid user ID in token".to_string(),
            })
        }
    };

    // 构建动态 UPDATE — 仅更新提供的字段；占位符按实际绑定序顺排
    // （修复：原固定 $1/$2/$3 与条件绑定错位——单字段提交必 500，
    // 路由双前缀修复前该路径不可达故从未暴露）
    // 显示名唯一落点 = `display_name`（`name` 为 DTO 兼容字段，同源写入；
    // 二者同时给出时以 `display_name` 为准，避免同列重复赋值）
    let display_name_value: Option<&String> = body.display_name.as_ref().or(body.name.as_ref());
    let mut set_clauses: Vec<String> = Vec::new();
    if display_name_value.is_some() {
        set_clauses.push(format!("display_name = ${}", set_clauses.len() + 1));
    }

    if set_clauses.is_empty() {
        return HttpResponse::BadRequest().json(AuthError {
            error: "No fields to update".to_string(),
        });
    }
    set_clauses.push("updated_at = NOW()".to_string());

    let sql = format!(
        "UPDATE isahl_auth.auth_users SET {} WHERE id = ${}",
        set_clauses.join(", "),
        set_clauses.len()
    );

    let mut query = sqlx::query(AssertSqlSafe(sql.as_str()));
    if let Some(display_name) = display_name_value {
        query = query.bind(display_name);
    }
    query = query.bind(user_id);

    if let Err(e) = query.execute(pool.get_ref()).await {
        log::error!("Failed to update profile for user {}: {}", user_id, e);
        return HttpResponse::InternalServerError().json(AuthError {
            error: "Failed to update profile".to_string(),
        });
    }

    log::info!("Profile updated for user {}", user_id);

    HttpResponse::Ok().json(serde_json::json!({
        "message": "Profile updated successfully"
    }))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/me").route("/profile", web::patch().to(update_profile)));
}
