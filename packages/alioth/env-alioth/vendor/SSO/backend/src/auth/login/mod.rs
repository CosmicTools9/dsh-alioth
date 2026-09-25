//! Login, logout, and registration handlers
//!
//! Provides HTTP handlers for authentication flows including:
//! - User registration
//! - Login with optional MFA
//! - MFA verification step
//! - Logout
//! - Token refresh
//! - Session management

use actix_web::HttpRequest;
use serde::{Deserialize, Serialize};

mod handlers;
mod sessions;
mod tokens;

pub use handlers::{configure, login, login_mfa, logout, me, refresh};
pub(crate) use handlers::{issue_login_response, status_gate_error};
pub use sessions::{list_sessions, revoke_other_sessions, revoke_session};
pub(crate) use tokens::is_valid_refresh_token;
pub(crate) use tokens::purge_expired_tokens;
pub use tokens::{record_failed_login, reset_failed_login};

/// Registration request body
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

/// Registration response
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user_id: String,
    pub email: String,
}

/// Login request body - supports email/username/phone auto-detection
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    #[serde(alias = "username")]
    pub identifier: String,
    pub password: String,
}

/// 择账号候选（`username_selection_required = true` 时非空）
#[derive(Debug, Serialize)]
pub struct LoginCandidate {
    pub username: String,
    pub display_name: Option<String>,
}

/// Login response (before MFA)
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub mfa_required: bool,
    pub message: Option<String>,
    pub session_id: Option<String>,
    /// 多账号共享标识（email/phone）时的择账号挑战：true 表示本次响应**未**签发令牌、
    /// **未**建立会话，客户端 MUST 让用户从 `candidates` 选 username 后以该 username 重新登录
    /// （allow-duplicate-email-accounts）。
    pub username_selection_required: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<LoginCandidate>,
}

/// MFA login request body
#[derive(Debug, Deserialize)]
pub struct MfaLoginRequest {
    pub email: String,
    pub code: String,
    pub session_id: Option<String>,
}

/// MFA login response (after MFA verification)
#[derive(Debug, Serialize)]
pub struct MfaLoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: String,
}

/// Token refresh response
#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct AuthError {
    pub error: String,
}

/// Get client IP from request（转调共享实现 `auth::session::client_ip`——**可信跳才采信转发头**；
/// 本文件曾有私有复制，2026-09-23 fix-auth-ratelimit-client-ip 统一口径）
fn get_client_ip(req: &HttpRequest) -> Option<String> {
    crate::auth::session::client_ip(req)
}

/// Get user agent from request
fn get_user_agent(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // portal-scope 推导的分支覆盖统一在 `auth::portal::portal_scope_tests`（唯一实现处）
    // ——本文件曾内嵌一份与生产逻辑已漂移的测试副本（空 attrs → 空 scope，而生产为
    // workbench 默认），2026-09-24 删除。

    // ── 失败登录计数 / 锁定集成测试（需测试库） ──────────────────────────────────
    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
            .unwrap_or_else(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "william.d.zk".to_string());
                format!("postgres://{}@localhost:5432/aliothstudio_test", user)
            });
        sqlx::PgPool::connect(&url)
            .await
            .expect("无法连接测试库，请先运行 `bash scripts/db/reset-db.sh --test`")
    }

    #[tokio::test]
    async fn test_record_failed_login_increments_and_locks() {
        let pool = test_pool().await;
        let suffix = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let user_id: i64 = sqlx::query_scalar(
            "INSERT INTO isahl_auth.auth_users (name, email, password_hash, status) \
             VALUES ($1, $2, 'argon2-placeholder', 'active') RETURNING id",
        )
        .bind(format!("lf_test_{}", suffix))
        .bind(format!("lf_test_{}@example.com", suffix))
        .fetch_one(&pool)
        .await
        .expect("插入测试用户失败");

        // 连续 5 次失败：current_attempts 0..4，第 5 次达到阈值并锁定
        for cur in 0..5i32 {
            record_failed_login(&pool, user_id, cur).await;
        }

        let (attempts, locked): (i32, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
            "SELECT failed_login_attempts, locked_until FROM isahl_auth.auth_users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("查询失败登录计数失败");

        assert_eq!(attempts, 5, "连续 5 次失败应达到阈值");
        assert!(locked.is_some(), "达到阈值应被锁定（locked_until 非空）");

        // 清理
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await;
    }
}
