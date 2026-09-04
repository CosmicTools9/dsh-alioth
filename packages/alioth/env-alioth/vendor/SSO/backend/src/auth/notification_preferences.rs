//! 用户通知偏好管理 handlers
//!
//! GET  /auth/me/notification-preferences   — 读取当前用户的通知偏好
//! PUT  /auth/me/notification-preferences   — 写入当前用户的通知偏好
//!
//! 偏好以 JSONB 形式持久化在 `auth_users.notification_preferences` 列
//! （迁移 016_add_notification_preferences.sql 创建）。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::jwt::{decode_token_any, Claims};
use super::login::AuthError;

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct NotificationPreferences {
    /// 是否接收审批通知
    pub approval_enabled: Option<bool>,
    /// 是否接收站内信（IM）通知
    pub im_enabled: Option<bool>,
    /// 是否接收邮件通知
    pub email_enabled: Option<bool>,
    /// 是否接收日程提醒
    pub schedule_enabled: Option<bool>,
    /// 是否接收系统公告
    pub announcement_enabled: Option<bool>,
    /// 免打扰时间窗（24h 格式）
    pub quiet_hours_start: Option<String>,
    pub quiet_hours_end: Option<String>,
    /// 自定义偏好键值对（兜底）
    pub custom: Option<serde_json::Value>,
}

fn extract_user_id(req: &HttpRequest, public_keys: &[&[u8]]) -> Result<i64, HttpResponse> {
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
                return Err(HttpResponse::Unauthorized().json(AuthError {
                    error: "No authentication token".to_string(),
                }))
            }
        },
    };
    let claims: Claims = match decode_token_any(&access_token, public_keys) {
        Ok(c) => c,
        Err(_) => {
            return Err(HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or expired token".to_string(),
            }))
        }
    };
    match claims.sub.parse::<i64>() {
        Ok(id) => Ok(id),
        Err(_) => Err(HttpResponse::Unauthorized().json(AuthError {
            error: "Invalid user ID in token".to_string(),
        })),
    }
}

/// 从 auth_users.notification_preferences (jsonb 字段) 读取通知偏好。
async fn read_prefs(
    pool: &PgPool,
    user_id: i64,
) -> Result<Option<NotificationPreferences>, String> {
    let raw: Option<Option<serde_json::Value>> = sqlx::query_scalar(
        "SELECT NULLIF(notification_preferences, '{}'::jsonb)
         FROM isahl_auth.auth_users
         WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    match raw {
        Some(Some(value)) => Ok(Some(
            serde_json::from_value::<NotificationPreferences>(value)
                .map_err(|e| format!("Decode error: {}", e))?,
        )),
        _ => Ok(None),
    }
}

/// GET /auth/me/notification-preferences
pub async fn get_preferences(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<super::AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state.verification_keys()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    match read_prefs(pool.get_ref(), user_id).await {
        Ok(Some(prefs)) => HttpResponse::Ok().json(prefs),
        Ok(None) => HttpResponse::Ok().json(NotificationPreferences::default()),
        Err(e) => {
            log::error!("read_prefs: {}", e);
            HttpResponse::InternalServerError().json(AuthError { error: e })
        }
    }
}

/// PUT /auth/me/notification-preferences
pub async fn put_preferences(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<super::AuthState>,
    body: web::Json<NotificationPreferences>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state.verification_keys()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    let prefs_value = match serde_json::to_value(&body.0) {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::BadRequest().json(AuthError {
                error: format!("Invalid preferences payload: {}", e),
            })
        }
    };

    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.auth_users
         SET notification_preferences = $1, updated_at = NOW()
         WHERE id = $2",
    )
    .bind(&prefs_value)
    .bind(user_id)
    .execute(pool.get_ref())
    .await
    {
        log::error!("Failed to write notification preferences: {}", e);
        return HttpResponse::InternalServerError().json(AuthError {
            error: format!("Failed to save preferences: {}", e),
        });
    }

    HttpResponse::Ok().json(body.0)
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth/me/notification-preferences")
            .route("", web::get().to(get_preferences))
            .route("", web::put().to(put_preferences)),
    );
}
