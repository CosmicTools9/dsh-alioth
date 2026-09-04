//! Admin 会话管理 handlers

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Session response (admin view)
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SessionListItem {
    pub session_token: String,
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub status: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_activity_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

use super::require_admin;
use crate::auth::AuthState;

// ============================================================================
// Sessions
// ============================================================================

/// GET /api/admin/sessions?user_id={id}
/// List active sessions for a user (max 200).
pub async fn list_user_sessions(
    req: HttpRequest,
    query: web::Query<SessionListQuery>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    if query.user_id == 0 {
        return HttpResponse::BadRequest().json(serde_json::json!({"error": "user_id required"}));
    }

    let rows = sqlx::query_as::<_, SessionListItem>(
        r#"
        SELECT session_token, user_id, status, ip_address, user_agent,
               created_at, last_activity_at, expires_at
        FROM isahl_auth.sso_sessions
        WHERE user_id = $1
        ORDER BY created_at DESC
        LIMIT 200
        "#,
    )
    .bind(query.user_id)
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(sessions) => HttpResponse::Ok().json(serde_json::json!({"sessions": sessions})),
        Err(e) => {
            log::error!("list_user_sessions DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list sessions"}))
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct SessionListQuery {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
}

/// DELETE /api/admin/sessions/{token}
/// Revoke a session by its token.
pub async fn revoke_user_session(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let token = path.into_inner();
    let result = sqlx::query(
        "UPDATE isahl_auth.sso_sessions SET status = 'revoked', updated_at = NOW() WHERE session_token = $1 AND status = 'active'",
    )
    .bind(&token)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "revoked"}))
        }
        Ok(_) => HttpResponse::NotFound()
            .json(serde_json::json!({"error": "Session not found or already revoked"})),
        Err(e) => {
            log::error!("revoke_user_session DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to revoke session"}))
        }
    }
}
