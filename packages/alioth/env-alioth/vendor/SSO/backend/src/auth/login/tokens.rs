//! Refresh-token 持久化与失败登录计数（自原 login.rs 纯拆分，零行为变化）。

use sqlx::PgPool;

/// Hash refresh token using SHA-256
fn hash_refresh_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Store refresh token in database
///
/// 不指定 `id`，由 `isahl.gen_next_zuid()` 生成，避免并发/时间戳冲突
/// 导致主键重复而使刷新令牌无法持久化。
pub(super) async fn store_refresh_token(
    pool: &PgPool,
    user_id: i64,
    token: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    let token_hash = hash_refresh_token(token);

    // 编译期宏 query! 会读 SSO/backend/.env 的 enc:// 加密 URL（无 sqlx driver）→ 构建失败；
    // 改运行时 query（与全仓一致，不依赖编译期 DB 连接）
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.refresh_tokens (user_id, token_hash, expires_at, created_at)
        VALUES ($1, $2, $3, NOW())
        ON CONFLICT (token_hash) DO UPDATE SET expires_at = $3
        "#,
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// Revoke refresh token
pub(super) async fn revoke_refresh_token(pool: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    let token_hash = hash_refresh_token(token);
    sqlx::query(
        r#"
        UPDATE isahl_auth.refresh_tokens
        SET revoked = TRUE
        WHERE token_hash = $1
        "#,
    )
    .bind(token_hash)
    .execute(pool)
    .await?;

    Ok(())
}

/// Validate refresh token against database
///
/// 供本模块 refresh 流程与 `auth::zchat` refresh grant 复用（经 `auth/mod.rs` 导出），
/// 保证 zchat 通道与主登录通道使用同一份 DB 哈希 + 吊销 + 过期校验实现。
pub(crate) async fn is_valid_refresh_token(
    pool: &PgPool,
    token: &str,
) -> Result<bool, sqlx::Error> {
    let token_hash = hash_refresh_token(token);

    let result: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM isahl_auth.refresh_tokens
            WHERE token_hash = $1 AND revoked = FALSE AND expires_at > NOW()
        )
        "#,
    )
    .bind(token_hash)
    .fetch_one(pool)
    .await?;

    Ok(result)
}

/// Revoke all user refresh tokens (for logout)
pub(super) async fn revoke_all_user_tokens(pool: &PgPool, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE isahl_auth.refresh_tokens
        SET revoked = TRUE
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Record a failed login attempt, locking the account after `MAX_FAILED_ATTEMPTS`
/// (5) consecutive failures for `LOCKOUT_MINUTES` (15) (SECURITY_SPEC §5).
///
/// Uses a runtime query (no compile-time schema dependency) so it stays valid
/// across the lockout-migration boundary.
pub async fn record_failed_login(pool: &PgPool, user_id: i64, current_attempts: i32) {
    const MAX_FAILED_ATTEMPTS: i32 = 5;
    const LOCKOUT_MINUTES: i32 = 15;

    let new_attempts = current_attempts.saturating_add(1);
    let sql = if new_attempts >= MAX_FAILED_ATTEMPTS {
        "UPDATE isahl_auth.auth_users \
         SET failed_login_attempts = $1, locked_until = NOW() + INTERVAL '15 minutes' \
         WHERE id = $2"
    } else {
        "UPDATE isahl_auth.auth_users SET failed_login_attempts = $1 WHERE id = $2"
    };
    let _ = sqlx::query(sql)
        .bind(new_attempts)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| log::error!("Failed to record failed login for user {}: {}", user_id, e));
    if new_attempts >= MAX_FAILED_ATTEMPTS {
        log::warn!(
            "Account {} locked for {} minutes after {} failed attempts",
            user_id,
            LOCKOUT_MINUTES,
            MAX_FAILED_ATTEMPTS
        );
    }
}

/// Reset login failure counter and clear any lockout on successful authentication.
pub async fn reset_failed_login(pool: &PgPool, user_id: i64) {
    let _ = sqlx::query(
        "UPDATE isahl_auth.auth_users \
         SET failed_login_attempts = 0, locked_until = NULL \
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| log::error!("Failed to reset failed login for user {}: {}", user_id, e));
}
