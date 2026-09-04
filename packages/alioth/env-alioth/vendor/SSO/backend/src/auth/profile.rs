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
    let access_token = match req.cookie("access_token") {
        Some(c) => c.value().to_string(),
        None => match req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|auth| auth.strip_prefix("Bearer "))
        {
            Some(token) => token.to_string(),
            None => {
                return HttpResponse::Unauthorized().json(AuthError {
                    error: "No authentication token".to_string(),
                })
            }
        },
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

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid user ID in token".to_string(),
            })
        }
    };

    // 构建动态 UPDATE — 仅更新提供的字段
    let mut set_clauses: Vec<String> = Vec::new();
    if body.name.is_some() {
        set_clauses.push("name = $1".to_string());
    }
    if body.display_name.is_some() {
        set_clauses.push("display_name = $2".to_string());
    }
    set_clauses.push("updated_at = NOW()".to_string());

    if set_clauses.is_empty() {
        return HttpResponse::BadRequest().json(AuthError {
            error: "No fields to update".to_string(),
        });
    }

    let sql = format!(
        "UPDATE isahl_auth.auth_users SET {} WHERE id = $3",
        set_clauses.join(", ")
    );

    let mut query = sqlx::query(AssertSqlSafe(sql.as_str()));
    if let Some(ref name) = body.name {
        query = query.bind(name);
    }
    if let Some(ref display_name) = body.display_name {
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
    cfg.service(web::scope("/auth/me").route("/profile", web::patch().to(update_profile)));
}
