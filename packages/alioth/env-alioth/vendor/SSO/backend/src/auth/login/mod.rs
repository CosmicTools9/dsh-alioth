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
pub use sessions::{list_sessions, revoke_other_sessions, revoke_session};
pub(crate) use tokens::is_valid_refresh_token;
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

/// Login response (before MFA)
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub mfa_required: bool,
    pub message: Option<String>,
    pub session_id: Option<String>,
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

/// Application state for auth handlers
/// Get client IP from request
fn get_client_ip(req: &HttpRequest) -> Option<String> {
    req.connection_info()
        .realip_remote_addr()
        .map(|s| s.to_string())
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

    /// portal-scope 推导逻辑（与 me handler 内联逻辑一致）
    fn derive_portal_scope_for_test(attrs: &[String]) -> (Vec<String>, String) {
        let has_workbench = attrs.iter().any(|a| a == "admin" || a == "operator");
        let has_storefront = attrs
            .iter()
            .any(|a| a == "user" || a == "customer" || a == "storefront");
        let mut portal_scope: Vec<String> = Vec::new();
        if has_workbench {
            portal_scope.push("workbench".to_string());
        }
        if has_storefront {
            portal_scope.push("storefront".to_string());
        }
        // 有 NGAC 属性但仍未推导出 scope 时默认 workbench
        if portal_scope.is_empty() && !attrs.is_empty() {
            portal_scope.push("workbench".to_string());
        }
        let portal_default = if has_storefront && !has_workbench {
            "storefront"
        } else {
            "workbench"
        };
        (portal_scope, portal_default.to_string())
    }

    #[test]
    fn test_empty_attrs_gets_empty_scope() {
        // 无 NGAC 属性时 !ngac_attrs.is_empty() 门禁阻止默认 scope
        let (scope, _) = derive_portal_scope_for_test(&[]);
        assert!(
            scope.is_empty(),
            "empty attrs should get empty scope, got {:?}",
            scope
        );
    }

    #[test]
    fn test_storefront_only() {
        let (scope, default) = derive_portal_scope_for_test(&["user".into()]);
        assert!(scope.contains(&"storefront".to_string()));
        assert!(!scope.contains(&"workbench".to_string()));
        assert_eq!(default, "storefront");
    }

    #[test]
    fn test_admin_gets_workbench() {
        let (scope, default) = derive_portal_scope_for_test(&["admin".into()]);
        assert!(scope.contains(&"workbench".to_string()));
        assert_eq!(default, "workbench");
    }

    #[test]
    fn test_operator_gets_workbench() {
        let (scope, default) = derive_portal_scope_for_test(&["operator".into()]);
        assert!(scope.contains(&"workbench".to_string()));
        assert_eq!(default, "workbench");
    }

    #[test]
    fn test_unknown_attr_defaults_to_workbench() {
        // viewer 等未知属性 — 有 NGAC 属性但无匹配 scope → 默认 workbench
        let (scope, _) = derive_portal_scope_for_test(&["viewer".into()]);
        assert!(
            scope.contains(&"workbench".to_string()),
            "unknown attrs should get workbench, got {:?}",
            scope
        );
    }

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
