//! SSO Session Management
//!
//! Provides centralized session storage for single sign-on (SSO) and
//! single logout (SLO) functionality.
//!
//! # Features
//!
//! - Centralized session storage in PostgreSQL
//! - Session validation and lifecycle management
//! - Multi-device session tracking
//! - Session revocation (logout)
//! - Automatic cleanup of expired sessions
//!
//! # Design Decision: UUID for Session IDs
//!
//! **Note:** SSO Sessions use `UUID` (not `BIGINT`) for the following reasons:
//!
//! 1. **External IdP Compatibility**: Session IDs may be shared with external Identity
//!    Providers (OIDC) which typically use UUID/GUID format
//! 2. **Security**: UUIDs are unguessable, making session enumeration attacks impractical
//! 3. **Distributed Systems**: UUIDs can be generated independently across multiple
//!    SSO nodes without coordination
//!
//! This is an intentional design decision that differs from the standard `BIGINT`
//! primary key used in business tables per DDL_DESIGN_SPEC.md.
//!
//! Related tables (slo_notifications, session_revocations) also use UUID to maintain
//! referential integrity with sso_sessions.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

/// SSO Session status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// Session is active and valid
    Active,
    /// Session has expired
    Expired,
    /// Session was revoked (logout)
    Revoked,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::Expired => write!(f, "expired"),
            SessionStatus::Revoked => write!(f, "revoked"),
        }
    }
}

// sqlx type mapping: DB TEXT <-> Rust SessionStatus
impl sqlx::Type<sqlx::Postgres> for SessionStatus {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        sqlx::postgres::PgTypeInfo::with_name("TEXT")
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for SessionStatus {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let s = <&str as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        match s {
            "active" => Ok(SessionStatus::Active),
            "expired" => Ok(SessionStatus::Expired),
            "revoked" => Ok(SessionStatus::Revoked),
            _ => {
                let err: Box<dyn std::error::Error + Send + Sync> = Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown session status: {}", s),
                ));
                Err(err)
            }
        }
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for SessionStatus {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        let s = match self {
            SessionStatus::Active => "active",
            SessionStatus::Expired => "expired",
            SessionStatus::Revoked => "revoked",
        };
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&s, buf)
    }
}

/// SSO Session representation
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SsoSession {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub session_token: String,
    pub refresh_token_hash: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub idp_provider_id: Option<i64>,
    pub idp_session_id: Option<String>,
    pub status: SessionStatus,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub front_channel_logout_uri: Option<String>,
    pub back_channel_logout_uri: Option<String>,
}

/// Request to create a new session
#[derive(Debug, Clone, Default)]
pub struct CreateSessionRequest {
    pub user_id: i64,
    pub idp_provider_id: Option<i64>,
    pub idp_session_id: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub front_channel_logout_uri: Option<String>,
    pub back_channel_logout_uri: Option<String>,
    pub refresh_token_hash: Option<String>,
}

/// 客户端 IP（各登录链 create_session 共用；ldap/webauthn 曾有私有复制，
/// fix-sso-noauth-removal 起新代码统一走本函数）。
///
/// **只有可信跳才采信转发头**（2026-09-23 修复 fix-auth-ratelimit-client-ip）：
/// 本服务所有上游（nginx / Gateway / 各前端静态服务）都与它同机，故仅当 TCP 对端是**回环地址**
/// 时才读 `X-Forwarded-For`（左起首个）/ `X-Real-IP`；其余对端一律用 TCP 对端地址。
///
/// 旧实现无条件读 `realip_remote_addr()`，实测两处后果（两机复现）：
/// ① **折叠**——平台链路（浏览器 → FE 服务器 → Gateway → SSO）各跳都不设转发头 ⇒
///    全部远程用户折进 `127.0.0.1` 同一个限流桶，`auth` 档 10 次/60 秒成了**全平台共享额度**，
///    该桶实测一分钟计数 190–262，任何人多试几次就把别人一起顶到 429；
/// ② **可伪造**——任何客户端自带 `X-Forwarded-For: 1.2.3.4` 即被原样落库（限流无限开桶）。
pub(crate) fn client_ip(req: &actix_web::HttpRequest) -> Option<String> {
    client_ip_from_conn_info(&req.connection_info())
}

/// [`client_ip`] 的核心：只依赖 `ConnectionInfo`，故**中间件**（`ServiceRequest`）可直接复用
/// `req.connection_info()` 调用同一判定，无需各自复制口径。
pub(crate) fn client_ip_from_conn_info(conn: &actix_web::dev::ConnectionInfo) -> Option<String> {
    resolve_client_ip(
        conn.peer_addr().and_then(parse_ip),
        conn.realip_remote_addr(),
    )
}

/// `ConnectionInfo::peer_addr()` 的形态是 `"ip:port"`（个别内核/代理下为裸 `ip`）：两种都认。
fn parse_ip(raw: &str) -> Option<std::net::IpAddr> {
    raw.parse::<std::net::IpAddr>()
        .ok()
        .or_else(|| raw.parse::<std::net::SocketAddr>().ok().map(|a| a.ip()))
}

/// [`client_ip`] 的纯判定（可单测）：对端非回环 ⇒ 只用对端；对端回环 ⇒ 采信转发值，缺失回退对端。
fn resolve_client_ip(peer_ip: Option<std::net::IpAddr>, forwarded: Option<&str>) -> Option<String> {
    match peer_ip {
        Some(ip) if !ip.is_loopback() => Some(ip.to_string()),
        Some(ip) => forwarded
            .map(|s| s.to_string())
            .or_else(|| Some(ip.to_string())),
        None => forwarded.map(|s| s.to_string()),
    }
}

/// 客户端 User-Agent（同上）。
pub(crate) fn client_user_agent(req: &actix_web::HttpRequest) -> Option<String> {
    req.headers()
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
}

/// Session errors
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("Session not found")]
    NotFound,
    #[error("Session has expired")]
    Expired,
    #[error("Session has been revoked")]
    Revoked,
    #[error("Invalid session token")]
    InvalidToken,
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
}

/// Session manager for centralized session handling
#[derive(Debug, Clone)]
pub struct SessionManager {
    pool: PgPool,
    /// Default session duration in days
    session_duration_days: i64,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            session_duration_days: 7,
        }
    }

    /// Create a new session manager with custom session duration
    pub fn with_duration(pool: PgPool, days: i64) -> Self {
        Self {
            pool,
            session_duration_days: days,
        }
    }

    /// Create a new SSO session
    pub async fn create_session(
        &self,
        req: CreateSessionRequest,
    ) -> Result<SsoSession, SessionError> {
        let session_token = generate_session_token();
        let expires_at = Utc::now() + Duration::days(self.session_duration_days);

        let session = sqlx::query_as::<_, SsoSession>(
            r#"
            INSERT INTO isahl_auth.sso_sessions (
                user_id, session_token, refresh_token_hash,
                idp_provider_id, idp_session_id, status,
                ip_address, user_agent, expires_at,
                front_channel_logout_uri, back_channel_logout_uri
            )
            VALUES ($1, $2, $3, $4, $5, 'active', $6, $7, $8, $9, $10)
            RETURNING *
            "#,
        )
        .bind(req.user_id)
        .bind(&session_token)
        .bind(req.refresh_token_hash)
        .bind(req.idp_provider_id)
        .bind(req.idp_session_id)
        .bind(req.ip_address)
        .bind(req.user_agent)
        .bind(expires_at)
        .bind(req.front_channel_logout_uri)
        .bind(req.back_channel_logout_uri)
        .fetch_one(&self.pool)
        .await?;

        Ok(session)
    }

    /// Validate a session by token
    /// Returns the session if valid, or an error if invalid/expired/revoked
    pub async fn validate_session(&self, session_token: &str) -> Result<SsoSession, SessionError> {
        let session = self.get_session(session_token).await?;

        // Check if session is active
        match session.status {
            SessionStatus::Revoked => return Err(SessionError::Revoked),
            SessionStatus::Expired => return Err(SessionError::Expired),
            SessionStatus::Active => {}
        }

        // Check expiration
        if session.expires_at < Utc::now() {
            // Auto-update status to expired
            self.update_session_status(&session.session_token, SessionStatus::Expired)
                .await?;
            return Err(SessionError::Expired);
        }

        // Update last activity
        self.update_last_activity(&session.session_token).await?;

        Ok(session)
    }

    /// Get a session by token (without validation)
    pub async fn get_session(&self, session_token: &str) -> Result<SsoSession, SessionError> {
        let session = sqlx::query_as::<_, SsoSession>(
            r#"
            SELECT * FROM isahl_auth.sso_sessions
            WHERE session_token = $1
            "#,
        )
        .bind(session_token)
        .fetch_optional(&self.pool)
        .await?;

        session.ok_or(SessionError::NotFound)
    }

    /// Revoke a session (logout)
    pub async fn revoke_session(
        &self,
        session_token: &str,
        revoked_by: Option<i64>,
        reason: &str,
    ) -> Result<(), SessionError> {
        let session = self.get_session(session_token).await?;

        // Update session status
        sqlx::query(
            r#"
            UPDATE isahl_auth.sso_sessions
            SET status = 'revoked'
            WHERE session_token = $1
            "#,
        )
        .bind(session_token)
        .execute(&self.pool)
        .await?;

        // Record revocation for audit
        sqlx::query(
            r#"
            INSERT INTO isahl_auth.session_revocations (
                session_id, user_id, revoked_by, revocation_reason
            )
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(session.id)
        .bind(session.user_id)
        .bind(revoked_by)
        .bind(reason)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Revoke all sessions for a user
    /// Returns the number of sessions revoked
    pub async fn revoke_all_user_sessions(
        &self,
        user_id: i64,
        except: Option<&str>,
        revoked_by: Option<i64>,
        reason: &str,
    ) -> Result<u64, SessionError> {
        // Get sessions to revoke
        let sessions = if let Some(except_token) = except {
            sqlx::query_as::<_, (i64, String)>(
                r#"
                SELECT id, session_token FROM isahl_auth.sso_sessions
                WHERE user_id = $1 AND status = 'active' AND session_token != $2
                "#,
            )
            .bind(user_id)
            .bind(except_token)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, (i64, String)>(
                r#"
                SELECT id, session_token FROM isahl_auth.sso_sessions
                WHERE user_id = $1 AND status = 'active'
                "#,
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?
        };

        let count = sessions.len() as u64;

        // Revoke each session
        for (session_id, _session_token) in sessions {
            // Update status
            sqlx::query(
                r#"
                UPDATE isahl_auth.sso_sessions
                SET status = 'revoked'
                WHERE id = $1
                "#,
            )
            .bind(session_id)
            .execute(&self.pool)
            .await?;

            // Record revocation
            sqlx::query(
                r#"
                INSERT INTO isahl_auth.session_revocations (
                    session_id, user_id, revoked_by, revocation_reason
                )
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(session_id)
            .bind(user_id)
            .bind(revoked_by)
            .bind(reason)
            .execute(&self.pool)
            .await?;
        }

        Ok(count)
    }

    /// List all active sessions for a user
    pub async fn list_user_sessions(&self, user_id: i64) -> Result<Vec<SsoSession>, SessionError> {
        let sessions = sqlx::query_as::<_, SsoSession>(
            r#"
            SELECT * FROM isahl_auth.sso_sessions
            WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(sessions)
    }

    /// List active sessions for a user
    pub async fn list_active_sessions(
        &self,
        user_id: i64,
    ) -> Result<Vec<SsoSession>, SessionError> {
        let sessions = sqlx::query_as::<_, SsoSession>(
            r#"
            SELECT * FROM isahl_auth.sso_sessions
            WHERE user_id = $1 AND status = 'active'
            ORDER BY last_activity_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(sessions)
    }

    /// Get session count for a user
    pub async fn get_session_count(&self, user_id: i64) -> Result<i64, SessionError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) FROM isahl_auth.sso_sessions
            WHERE user_id = $1 AND status = 'active'
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Cleanup expired sessions
    /// Returns the number of sessions marked as expired
    pub async fn cleanup_expired_sessions(&self) -> Result<u64, SessionError> {
        let result = sqlx::query(
            r#"
            UPDATE isahl_auth.sso_sessions
            SET status = 'expired'
            WHERE status = 'active' AND expires_at < NOW()
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// 物理删除超保留期的 expired/revoked 会话（fix-sso-auth-gaps G4）。
    /// 关联子表 `session_revocations`/`slo_notifications` 经 FK ON DELETE CASCADE 级联。
    /// 幂等；多实例并发执行安全。
    pub async fn purge_expired_sessions(&self, retention_days: i64) -> Result<u64, SessionError> {
        let result = sqlx::query(
            r#"
            DELETE FROM isahl_auth.sso_sessions
            WHERE status IN ('expired', 'revoked')
              AND expires_at < NOW() - ($1 * INTERVAL '1 day')
            "#,
        )
        .bind(retention_days)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Update session status
    async fn update_session_status(
        &self,
        session_token: &str,
        status: SessionStatus,
    ) -> Result<(), SessionError> {
        sqlx::query(
            r#"
            UPDATE isahl_auth.sso_sessions
            SET status = $1
            WHERE session_token = $2
            "#,
        )
        .bind(status.to_string())
        .bind(session_token)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update last activity timestamp
    async fn update_last_activity(&self, session_token: &str) -> Result<(), SessionError> {
        sqlx::query(
            r#"
            UPDATE isahl_auth.sso_sessions
            SET last_activity_at = NOW()
            WHERE session_token = $1
            "#,
        )
        .bind(session_token)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get session by ID
    pub async fn get_session_by_id(&self, session_id: i64) -> Result<SsoSession, SessionError> {
        let session = sqlx::query_as::<_, SsoSession>(
            r#"
            SELECT * FROM isahl_auth.sso_sessions
            WHERE id = $1
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;

        session.ok_or(SessionError::NotFound)
    }

    /// Revoke session by ID (for admin operations)
    pub async fn revoke_session_by_id(
        &self,
        session_id: i64,
        revoked_by: Option<i64>,
        reason: &str,
    ) -> Result<(), SessionError> {
        let session = self.get_session_by_id(session_id).await?;

        // Update session status
        sqlx::query(
            r#"
            UPDATE isahl_auth.sso_sessions
            SET status = 'revoked'
            WHERE id = $1
            "#,
        )
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        // Record revocation for audit
        sqlx::query(
            r#"
            INSERT INTO isahl_auth.session_revocations (
                session_id, user_id, revoked_by, revocation_reason
            )
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(session_id)
        .bind(session.user_id)
        .bind(revoked_by)
        .bind(reason)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

/// Generate a cryptographically secure session token
fn generate_session_token() -> String {
    use rand::RngExt;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    const TOKEN_LENGTH: usize = 64;

    let mut rng = rand::rng();
    let token: String = (0..TOKEN_LENGTH)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();

    format!("sess_{}", token)
}

/// 校验 SSO 会话是否仍活跃（供 SSO 自身中间件与 Gateway PEP 共用同一判定逻辑）。
///
/// 仅当 JWT 携带 `sid` 时调用；会话不存在 / 已吊销 / 已过期 / 查询失败均视为失效。
/// SSO 受保护端点（audit/NGAC/WebSocket）此前只验 JWT 不查会话状态，登出后仍可在
/// token 有效期内（15min）访问 —— 补此校验以闭合吊销链路。
pub async fn is_session_active(pool: &PgPool, sid: &str) -> bool {
    let result = sqlx::query_as::<_, (String,)>(
        "SELECT status FROM isahl_auth.sso_sessions WHERE session_token = $1 LIMIT 1",
    )
    .bind(sid)
    .fetch_optional(pool)
    .await;

    match result {
        Ok(Some((status,))) => status == "active",
        // 会话不存在或查询失败：保守拒绝。
        Ok(None) | Err(_) => false,
    }
}

/// Generate a logout token for SLO (signed with ES256 private key)
pub fn generate_logout_token(
    session_id: &str,
    private_key: &[u8],
) -> Result<String, jsonwebtoken::errors::Error> {
    use chrono::Utc;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct LogoutClaims {
        /// Session ID
        pub sid: String,
        /// Issued at
        pub iat: i64,
        /// Expiration (short-lived, 5 minutes)
        pub exp: i64,
        /// Token type
        pub typ: String,
    }

    let now = Utc::now();
    let claims = LogoutClaims {
        sid: session_id.to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::minutes(5)).timestamp(),
        typ: "logout".to_string(),
    };

    encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(private_key)?,
    )
}

/// Verify a logout token (with ES256 public key)
pub fn verify_logout_token(
    token: &str,
    public_key: &[u8],
) -> Result<String, jsonwebtoken::errors::Error> {
    verify_logout_token_any(token, &[public_key])
}

/// Verify a logout token trying each public key in order（轮换窗口多 key）。
pub fn verify_logout_token_any(
    token: &str,
    public_keys: &[&[u8]],
) -> Result<String, jsonwebtoken::errors::Error> {
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct LogoutClaims {
        pub sid: String,
        pub iat: i64,
        pub exp: i64,
        pub typ: String,
    }

    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;

    let mut first_err: Option<jsonwebtoken::errors::Error> = None;
    for key in public_keys {
        match decode::<LogoutClaims>(token, &DecodingKey::from_ec_pem(key)?, &validation) {
            Ok(td) => return Ok(td.claims.sid),
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    Err(first_err.unwrap_or_else(|| {
        jsonwebtoken::errors::Error::from(jsonwebtoken::errors::ErrorKind::InvalidToken)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 分桶键口径（2026-09-23 fix-auth-ratelimit-client-ip）：
    /// 仅**回环对端**（本机可信跳：nginx / Gateway / 前端静态服务）才采信转发头；
    /// 非回环对端自带转发头一律忽略（防伪造开无限桶）；对端不可得才退让转发值。
    #[test]
    fn resolve_client_ip_trusts_forwarded_only_from_loopback_peer() {
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let remote: std::net::IpAddr = "203.0.113.9".parse().unwrap();

        assert_eq!(
            resolve_client_ip(Some(loopback), Some("183.156.251.60")).as_deref(),
            Some("183.156.251.60"),
            "可信跳 + 转发头 ⇒ 采信转发值（平台链路折叠的修法）"
        );
        assert_eq!(
            resolve_client_ip(Some(loopback), None).as_deref(),
            Some("127.0.0.1"),
            "可信跳但无转发头 ⇒ 回退对端（真·本机调用）"
        );
        assert_eq!(
            resolve_client_ip(Some(remote), Some("1.2.3.4")).as_deref(),
            Some("203.0.113.9"),
            "非可信跳自带伪造头 ⇒ 忽略，只用对端"
        );
        assert_eq!(
            resolve_client_ip(None, Some("1.2.3.4")).as_deref(),
            Some("1.2.3.4"),
            "对端不可得 ⇒ 退让转发值（不臆造）"
        );
    }

    #[test]
    fn test_generate_session_token() {
        let token1 = generate_session_token();
        let token2 = generate_session_token();

        // Tokens should be unique
        assert_ne!(token1, token2);

        // Token should start with 'sess_'
        assert!(token1.starts_with("sess_"));

        // Token should be correct length (sess_ + 64 chars)
        assert_eq!(token1.len(), 69);
    }

    #[test]
    fn test_session_status_display() {
        assert_eq!(SessionStatus::Active.to_string(), "active");
        assert_eq!(SessionStatus::Expired.to_string(), "expired");
        assert_eq!(SessionStatus::Revoked.to_string(), "revoked");
    }

    // 测试用 EC P-256 密钥对（PKCS#8 PEM）
    const TEST_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs
\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0
\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm
\
-----END PRIVATE KEY-----";

    const TEST_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAExHBYWD4VZBXSBjQIgcMUKbtHGEV3
\
NK6CQd0RxdS3yLGgsXJ1XqdLJwuPvErVIsSI3ywGfDHPPrqmuN53XjRBZg==
\
-----END PUBLIC KEY-----";

    #[test]
    fn test_logout_token_generation() {
        let session_id = "test_session_123";

        let token = generate_logout_token(session_id, TEST_PRIVATE_KEY).unwrap();
        let decoded = verify_logout_token(&token, TEST_PUBLIC_KEY).unwrap();

        assert_eq!(decoded, session_id);
    }

    #[test]
    fn test_logout_token_expired() {
        use chrono::Utc;
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Serialize, Deserialize)]
        struct LogoutClaims {
            pub sid: String,
            pub iat: i64,
            pub exp: i64,
            pub typ: String,
        }

        let now = Utc::now();

        // Create an already expired token
        let claims = LogoutClaims {
            sid: "test_session".to_string(),
            iat: (now - Duration::hours(1)).timestamp(),
            exp: (now - Duration::minutes(30)).timestamp(),
            typ: "logout".to_string(),
        };

        let token = encode(
            &Header::new(Algorithm::ES256),
            &claims,
            &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY).unwrap(),
        )
        .unwrap();

        // Should fail validation
        let result = verify_logout_token(&token, TEST_PUBLIC_KEY);
        assert!(result.is_err());
    }
}
