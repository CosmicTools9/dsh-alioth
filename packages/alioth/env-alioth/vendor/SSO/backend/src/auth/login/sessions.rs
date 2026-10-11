//! 会话管理 handler（list/revoke/revoke-others，自原 login.rs 纯拆分，零行为变化）。

use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

use super::AuthError;
use crate::auth::{jwt::decode_token_any, session::SessionManager, AuthState};

/// Get user's sessions
pub async fn list_sessions(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    // Extract and validate access token（单一实现 = `common::csrf`；delta auth「不内联提取」）
    let access_token = match common::csrf::extract_token(&req) {
        Some(token) => token,
        None => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Missing authorization".to_string(),
            })
        }
    };
    let claims = match decode_token_any(&access_token, &state.verification_keys()) {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid token".to_string(),
            })
        }
    };

    // CSRF 自检（design.md §4「自检面」）：公开前缀下 cookie 鉴权端点（DELETE 会话），鉴权成功后校验
    if let Some(resp) = crate::auth::csrf_self_check(&req) {
        return resp;
    }

    let user_id = match claims.sub.parse::<i64>() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid user ID".to_string(),
            })
        }
    };

    let session_manager = SessionManager::new(pool.get_ref().clone());
    match session_manager.list_user_sessions(user_id).await {
        Ok(sessions) => HttpResponse::Ok().json(sessions),
        Err(e) => {
            log::error!("Failed to list sessions: {}", e);
            HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to list sessions".to_string(),
            })
        }
    }
}

/// Revoke a specific session
pub async fn revoke_session(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    path: web::Path<String>,
) -> HttpResponse {
    // Extract and validate access token（单一实现 = `common::csrf`；delta auth「不内联提取」）
    let access_token = match common::csrf::extract_token(&req) {
        Some(token) => token,
        None => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Missing authorization".to_string(),
            })
        }
    };
    let claims = match decode_token_any(&access_token, &state.verification_keys()) {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid token".to_string(),
            })
        }
    };

    // CSRF 自检（design.md §4「自检面」）：公开前缀下 cookie 鉴权端点（DELETE 会话），鉴权成功后校验
    if let Some(resp) = crate::auth::csrf_self_check(&req) {
        return resp;
    }

    let user_id = match claims.sub.parse::<i64>() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid user ID".to_string(),
            })
        }
    };

    let session_token = path.into_inner();
    let session_manager = SessionManager::new(pool.get_ref().clone());

    // Get session to verify ownership
    match session_manager.get_session(&session_token).await {
        Ok(session) => {
            // Only allow revoking own sessions
            if session.user_id != user_id {
                return HttpResponse::Forbidden().json(AuthError {
                    error: "Cannot revoke this session".to_string(),
                });
            }

            match session_manager
                .revoke_session(&session_token, Some(user_id), "user_action")
                .await
            {
                Ok(_) => HttpResponse::Ok().json(serde_json::json!({ "status": "revoked" })),
                Err(e) => {
                    log::error!("Failed to revoke session: {}", e);
                    HttpResponse::InternalServerError().json(AuthError {
                        error: "Failed to revoke session".to_string(),
                    })
                }
            }
        }
        Err(_) => HttpResponse::NotFound().json(AuthError {
            error: "Session not found".to_string(),
        }),
    }
}

/// Revoke all other sessions
pub async fn revoke_other_sessions(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    // Extract and validate access token（单一实现 = `common::csrf`；delta auth「不内联提取」）
    let access_token = match common::csrf::extract_token(&req) {
        Some(token) => token,
        None => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Missing authorization".to_string(),
            })
        }
    };
    let claims = match decode_token_any(&access_token, &state.verification_keys()) {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid token".to_string(),
            })
        }
    };

    // CSRF 自检（design.md §4「自检面」）：公开前缀下 cookie 鉴权端点（DELETE 会话），鉴权成功后校验
    if let Some(resp) = crate::auth::csrf_self_check(&req) {
        return resp;
    }

    let user_id = match claims.sub.parse::<i64>() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid user ID".to_string(),
            })
        }
    };

    // Get current session token to exclude
    let current_session = req
        .headers()
        .get("X-Session-Token")
        .and_then(|h| h.to_str().ok());

    let session_manager = SessionManager::new(pool.get_ref().clone());
    match session_manager
        .revoke_all_user_sessions(user_id, current_session, Some(user_id), "logout_other")
        .await
    {
        Ok(count) => HttpResponse::Ok().json(serde_json::json!({
            "status": "revoked",
            "count": count
        })),
        Err(e) => {
            log::error!("Failed to revoke sessions: {}", e);
            HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to revoke sessions".to_string(),
            })
        }
    }
}
