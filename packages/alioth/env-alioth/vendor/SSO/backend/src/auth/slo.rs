//! Single Logout (SLO) Implementation
//!
//! Provides Single Logout functionality for SSO, supporting both
//! Front-Channel and Back-Channel logout mechanisms.
//!
//! # Front-Channel Logout
//!
//! Uses hidden iframes to notify applications of logout.
//! The user's browser loads logout URLs in iframes.
//!
//! # Back-Channel Logout
//!
//! Server-to-server logout notifications via HTTP POST.
//! More reliable but requires applications to expose an endpoint.

use super::jwt::decode_token_any;
use super::session::{generate_logout_token, SessionError, SessionManager};
use super::AuthState;
use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// SLO errors
#[derive(Debug, thiserror::Error)]
pub enum SloError {
    #[error("Session not found")]
    SessionNotFound,
    #[error("Session error: {0}")]
    SessionError(#[from] SessionError),
    #[error("Invalid logout token")]
    InvalidLogoutToken,
    #[error("HTTP client error: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("JWT error: {0}")]
    JwtError(#[from] jsonwebtoken::errors::Error),
}

/// SLO result containing logout targets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloResult {
    /// URIs for front-channel logout (iframe-based)
    pub front_channel_logout_uris: Vec<String>,
    /// Targets for back-channel logout (server-to-server)
    pub back_channel_targets: Vec<BackChannelTarget>,
    /// Logout token for validation
    pub logout_token: String,
}

/// Back-channel logout target
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackChannelTarget {
    /// Service identifier
    pub service: String,
    /// Logout endpoint URL
    pub logout_uri: String,
    /// Logout token to send
    pub logout_token: String,
}

/// SLO notification record
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SloNotification {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub session_id: i64,
    pub target_service: String,
    pub notification_type: String,
    pub status: String,
    pub attempted_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error_message: Option<String>,
}

/// SLO handler for managing single logout
#[derive(Debug, Clone)]
pub struct SloHandler {
    pool: PgPool,
    auth_state: AuthState,
    http_client: reqwest::Client,
}

impl SloHandler {
    /// Create a new SLO handler
    pub fn new(pool: PgPool, auth_state: AuthState) -> Self {
        Self {
            pool,
            auth_state,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    /// Initiate single logout for a session
    /// This revokes the session and returns logout targets for other applications
    pub async fn initiate_slo(
        &self,
        session_token: &str,
        revoked_by: Option<i64>,
    ) -> Result<SloResult, SloError> {
        let session_manager = SessionManager::new(self.pool.clone());

        // Get the session first
        let session = session_manager.get_session(session_token).await?;

        // Generate logout token for this session
        let logout_token =
            generate_logout_token(&session.session_token, &self.auth_state.jwt_private_key)?;

        // Get all active sessions for this user (excluding current)
        let other_sessions = session_manager
            .list_active_sessions(session.user_id)
            .await?
            .into_iter()
            .filter(|s| s.session_token != session_token)
            .collect::<Vec<_>>();

        // Revoke all sessions
        for s in &other_sessions {
            session_manager
                .revoke_session(&s.session_token, revoked_by, "slo")
                .await?;
        }

        // Also revoke the initiating session
        session_manager
            .revoke_session(session_token, revoked_by, "slo_initiated")
            .await?;

        // Build SLO result
        let front_channel_logout_uris: Vec<String> = other_sessions
            .iter()
            .filter_map(|s| s.front_channel_logout_uri.clone())
            .map(|uri| format!("{}?logout_token={}", uri, logout_token))
            .collect();

        let back_channel_targets: Vec<BackChannelTarget> = other_sessions
            .iter()
            .filter_map(|s| {
                s.back_channel_logout_uri
                    .clone()
                    .map(|uri| BackChannelTarget {
                        service: s.session_token.clone(),
                        logout_uri: uri,
                        logout_token: logout_token.clone(),
                    })
            })
            .collect();

        // Record SLO notifications
        for target in &back_channel_targets {
            self.record_notification(session.id, &target.service, "back_channel", "pending")
                .await?;
        }

        Ok(SloResult {
            front_channel_logout_uris,
            back_channel_targets,
            logout_token,
        })
    }

    /// Handle front-channel logout request
    /// This is called when an iframe loads the logout endpoint
    pub async fn handle_front_channel_logout(
        &self,
        logout_token: &str,
    ) -> Result<HttpResponse, SloError> {
        // Verify the logout token
        let session_token = super::session::verify_logout_token_any(
            logout_token,
            &self.auth_state.verification_keys(),
        )
        .map_err(|_| SloError::InvalidLogoutToken)?;

        // Revoke the session
        let session_manager = SessionManager::new(self.pool.clone());
        session_manager
            .revoke_session(&session_token, None, "front_channel_logout")
            .await?;

        // Return a blank page that can be loaded in an iframe
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <script>
        // Notify parent window that logout is complete
        if (window.parent && window.parent !== window) {
            window.parent.postMessage({ type: 'slo:complete', status: 'success' }, '*');
        }
    </script>
</head>
<body></body>
</html>
"#;

        Ok(HttpResponse::Ok().content_type("text/html").body(html))
    }

    /// Handle back-channel logout request
    /// This is called by other applications via HTTP POST
    pub async fn handle_back_channel_logout(
        &self,
        logout_token: &str,
    ) -> Result<HttpResponse, SloError> {
        // Verify the logout token
        let session_token = super::session::verify_logout_token_any(
            logout_token,
            &self.auth_state.verification_keys(),
        )
        .map_err(|_| SloError::InvalidLogoutToken)?;

        // Revoke the session
        let session_manager = SessionManager::new(self.pool.clone());
        session_manager
            .revoke_session(&session_token, None, "back_channel_logout")
            .await?;

        Ok(HttpResponse::Ok().json(serde_json::json!({
            "status": "ok",
            "message": "Session revoked"
        })))
    }

    /// Send back-channel logout notification
    pub async fn send_back_channel_logout(
        &self,
        target: &BackChannelTarget,
    ) -> Result<(), SloError> {
        let response = self
            .http_client
            .post(&target.logout_uri)
            .json(&serde_json::json!({
                "logout_token": target.logout_token,
            }))
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(SloError::HttpError(
                response.error_for_status().unwrap_err(),
            ))
        }
    }

    /// Execute back-channel logout for all targets
    pub async fn execute_back_channel_logouts(
        &self,
        targets: &[BackChannelTarget],
    ) -> Vec<(String, Result<(), SloError>)> {
        let mut results = Vec::new();

        for target in targets {
            let result = self.send_back_channel_logout(target).await;
            results.push((target.service.clone(), result));
        }

        results
    }

    /// Record SLO notification in database
    async fn record_notification(
        &self,
        session_id: i64,
        target_service: &str,
        notification_type: &str,
        status: &str,
    ) -> Result<(), SloError> {
        sqlx::query(
            r#"
            INSERT INTO isahl_auth.slo_notifications (
                session_id, target_service, notification_type, status
            )
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(session_id)
        .bind(target_service)
        .bind(notification_type)
        .bind(status)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update notification status
    pub async fn update_notification_status(
        &self,
        notification_id: i64,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<(), SloError> {
        sqlx::query(
            r#"
            UPDATE isahl_auth.slo_notifications
            SET status = $1,
                completed_at = NOW(),
                error_message = $2
            WHERE id = $3
            "#,
        )
        .bind(status)
        .bind(error_message)
        .bind(notification_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get pending notifications
    pub async fn get_pending_notifications(
        &self,
        limit: i64,
    ) -> Result<Vec<SloNotification>, SloError> {
        let notifications = sqlx::query_as::<_, SloNotification>(
            r#"
            SELECT * FROM isahl_auth.slo_notifications
            WHERE status = 'pending'
            ORDER BY attempted_at ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(notifications)
    }
}

/// SLO logout request
#[derive(Debug, Deserialize)]
pub struct LogoutRequest {
    pub logout_token: String,
}

/// SLO logout response
#[derive(Debug, Serialize)]
pub struct LogoutResponse {
    pub status: String,
    pub front_channel_uris: Vec<String>,
}

/// Initiate SLO endpoint handler
pub async fn initiate_slo_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    // Get session token from header
    let session_token = match req
        .headers()
        .get("X-Session-Token")
        .and_then(|h| h.to_str().ok())
    {
        Some(token) => token.to_string(),
        None => {
            // Fallback: extract sid claim from access_token cookie
            match req.cookie("access_token") {
                Some(cookie) => {
                    let token = cookie.value().to_string();
                    match decode_token_any(&token, &state.verification_keys()) {
                        Ok(claims) if !claims.sid.is_empty() => claims.sid,
                        _ => {
                            return HttpResponse::BadRequest().json(serde_json::json!({
                                "error": "Missing session token"
                            }))
                        }
                    }
                }
                None => {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": "Missing session token"
                    }))
                }
            }
        }
    };

    let slo_handler = SloHandler::new(pool.get_ref().clone(), state.get_ref().clone());

    match slo_handler.initiate_slo(&session_token, None).await {
        Ok(result) => {
            // Execute back-channel logouts asynchronously
            let _ = slo_handler
                .execute_back_channel_logouts(&result.back_channel_targets)
                .await;

            HttpResponse::Ok().json(LogoutResponse {
                status: "initiated".to_string(),
                front_channel_uris: result.front_channel_logout_uris,
            })
        }
        Err(e) => {
            log::error!("SLO initiation failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to initiate logout"
            }))
        }
    }
}

/// Front-channel logout endpoint handler
pub async fn front_channel_logout_handler(
    query: web::Query<LogoutRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let slo_handler = SloHandler::new(pool.get_ref().clone(), state.get_ref().clone());

    match slo_handler
        .handle_front_channel_logout(&query.logout_token)
        .await
    {
        Ok(response) => response,
        Err(e) => {
            log::warn!("Front-channel logout failed: {}", e);
            HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Invalid logout token"
            }))
        }
    }
}

/// Back-channel logout endpoint handler
pub async fn back_channel_logout_handler(
    body: web::Json<LogoutRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let slo_handler = SloHandler::new(pool.get_ref().clone(), state.get_ref().clone());

    match slo_handler
        .handle_back_channel_logout(&body.logout_token)
        .await
    {
        Ok(response) => response,
        Err(e) => {
            log::warn!("Back-channel logout failed: {}", e);
            HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Invalid logout token"
            }))
        }
    }
}

/// Generate SLO logout page with iframes
pub async fn slo_logout_page(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    // Get session token from query or cookie
    let session_token = req
        .query_string()
        .split('&')
        .find(|p| p.starts_with("session_token="))
        .and_then(|p| p.split('=').nth(1))
        .map(|s| s.to_string())
        .or_else(|| {
            req.headers()
                .get("X-Session-Token")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string())
        });

    let session_token = match session_token {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest().content_type("text/html").body(
                r#"<!DOCTYPE html>
<html><body>
<h1>Error</h1>
<p>Missing session token</p>
</body></html>
"#,
            );
        }
    };

    let slo_handler = SloHandler::new(pool.get_ref().clone(), state.get_ref().clone());

    match slo_handler.initiate_slo(&session_token, None).await {
        Ok(result) => {
            // Build iframe HTML
            let iframes: String = result
                .front_channel_logout_uris
                .iter()
                .map(|uri| format!(r#"<iframe src="{}" style="display:none;" onload="onLogoutComplete()" onerror="onLogoutComplete()"></iframe>"#, uri))
                .collect::<Vec<_>>()
                .join("\n");

            let html = format!(
                r#"<!DOCTYPE html>
<html>
<head>
    <title>Logging out...</title>
    <script>
        let completed = 0;
        const total = {};
        
        function onLogoutComplete() {{
            completed++;
            if (completed >= total) {{
                // All logout requests completed
                window.location.href = '/auth/login?logout=success';
            }}
        }}
        
        // Timeout after 5 seconds
        setTimeout(function() {{
            window.location.href = '/auth/login?logout=success';
        }}, 5000);
    </script>
</head>
<body>
    <h1>Logging out...Please wait.</h1>
    {}
</body>
</html>
"#,
                result.front_channel_logout_uris.len(),
                iframes
            );

            HttpResponse::Ok().content_type("text/html").body(html)
        }
        Err(e) => {
            log::error!("SLO page generation failed: {}", e);
            HttpResponse::InternalServerError()
                .content_type("text/html")
                .body(
                    r#"<!DOCTYPE html>
<html><body>
<h1>Error</h1>
<p>Failed to process logout</p>
</body></html>
"#,
                )
        }
    }
}

use sqlx::FromRow;

/// Configure SLO routes
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth/slo")
            .route("/logout", web::get().to(slo_logout_page))
            .route("/initiate", web::post().to(initiate_slo_handler))
            .route(
                "/front-channel",
                web::get().to(front_channel_logout_handler),
            )
            .route("/back-channel", web::post().to(back_channel_logout_handler)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slo_result_serialization() {
        let result = SloResult {
            front_channel_logout_uris: vec!["https://app1.com/logout".to_string()],
            back_channel_targets: vec![BackChannelTarget {
                service: "app2".to_string(),
                logout_uri: "https://app2.com/api/logout".to_string(),
                logout_token: "token123".to_string(),
            }],
            logout_token: "main_token".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("front_channel_logout_uris"));
        assert!(json.contains("back_channel_targets"));
    }

    #[test]
    fn test_back_channel_target() {
        let target = BackChannelTarget {
            service: "test-service".to_string(),
            logout_uri: "https://example.com/logout".to_string(),
            logout_token: "test-token".to_string(),
        };

        assert_eq!(target.service, "test-service");
        assert_eq!(target.logout_uri, "https://example.com/logout");
    }
}
