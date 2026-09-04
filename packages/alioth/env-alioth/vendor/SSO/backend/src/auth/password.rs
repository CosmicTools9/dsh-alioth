//! Password hashing and verification
//!
//! Uses standard argon2id for password hashing.
//! Plain argon2 hashes without the `$argon2` prefix are treated as a legacy
//! format and automatically migrated to the standard `$argon2id$…` format on
//! successful login.
//!
//! NOTE: the older `aes256gcm:$argon2_hash` format (deterministic AES-GCM
//! nonce) has been removed — such hashes are no longer accepted.

use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use thiserror::Error;

/// Password hashing errors
#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("Failed to hash password")]
    HashError,
    #[error("Failed to verify password")]
    VerifyError,
    #[error("Invalid hash format")]
    InvalidHashFormat,
    #[error("Decryption failed")]
    DecryptionFailed,
}

// ============================================================================
// NEW STANDARD FORMAT: Direct argon2id hashing
// ============================================================================

/// Hash a password using standard argon2id.
/// The returned string starts with "$argon2id$".
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes())
        .map_err(|_| PasswordError::HashError)?
        .to_string()
        .pipe(Ok)
}

/// Verify a password against a stored hash.
///
/// Returns:
/// - `Ok(Some(hash))` where `hash` is the **canonical** hash string for this
///   password.  If the input was a legacy format the returned string is the
///   newly-computed standard hash, enabling transparent migration.
/// - `Ok(None)` – password does not match.
/// - `Err(_)` – hash format is corrupted.
pub fn verify_password(password: &str, hash: &str) -> Result<Option<String>, PasswordError> {
    // ------------------------------------------------------------------
    // 1. Already the new standard format ($argon2id$…)
    // ------------------------------------------------------------------
    if hash.starts_with("$argon2") {
        let parsed = PasswordHash::new(hash).map_err(|_| PasswordError::InvalidHashFormat)?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| PasswordError::VerifyError)?;
        return Ok(Some(hash.to_string()));
    }

    // ------------------------------------------------------------------
    // 2. Legacy format: plain argon2 without any prefix
    //    (auto-migrated to the standard `$argon2id$…` format on success)
    // ------------------------------------------------------------------
    // NOTE: the old `aes256gcm:$argon2_hash` format with its deterministic
    // AES-GCM nonce has been REMOVED and is no longer accepted.
    let parsed = PasswordHash::new(hash).map_err(|_| PasswordError::InvalidHashFormat)?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| PasswordError::VerifyError)?;
    Ok(Some(hash_password(password)?))
}

/// Returns `true` if the hash is **not** in the new standard argon2id format.
pub fn needs_migration(hash: &str) -> bool {
    !hash.starts_with("$argon2")
}

// ============================================================================
// 密码策略（SECURITY_SPEC §5 基线）
// ============================================================================

/// 密码策略校验错误
#[derive(Debug, thiserror::Error)]
pub enum PasswordPolicyError {
    #[error("Password must be at least {min} characters long")]
    TooShort { min: usize },
}

/// 校验密码基础策略（SECURITY_SPEC §5 基线：最小长度 8）。
///
/// 当前仅实施最小长度基线。复杂度 / 历史 / 过期规则默认不实施；
/// 若产品决定启用，须先修订 `SECURITY_SPEC.md` §5 并在迁移中落地对应列/表，
/// 再扩展本函数（保持单一校验入口，避免散落 `len() < 8`）。
pub fn validate_password_policy(password: &str) -> Result<(), PasswordPolicyError> {
    const MIN_LEN: usize = 8;
    if password.len() < MIN_LEN {
        return Err(PasswordPolicyError::TooShort { min: MIN_LEN });
    }
    Ok(())
}

/// Convenience helper: verify + migrate in one call.
/// Returns the new standard hash on success, `None` otherwise.
pub fn migrate_password_hash(old_hash: &str, password: &str) -> Option<String> {
    match verify_password(password, old_hash) {
        Ok(Some(new_hash)) if needs_migration(old_hash) => Some(new_hash),
        Ok(Some(_)) => Some(old_hash.to_string()),
        _ => None,
    }
}

// ============================================================================
// 异步包装：Argon2 是 CPU/内存密集型操作，必须在 blocking pool 执行，
// 否则会阻塞 tokio worker，导致 SSO 在并发请求下挂起超时。
// ============================================================================

/// 异步版本 hash_password，内部在 spawn_blocking 中执行 Argon2。
pub async fn hash_password_async(password: String) -> Result<String, PasswordError> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| PasswordError::HashError)?
}

/// 异步版本 verify_password，内部在 spawn_blocking 中执行 Argon2。
pub async fn verify_password_async(
    password: String,
    hash: String,
) -> Result<Option<String>, PasswordError> {
    tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .map_err(|_| PasswordError::VerifyError)?
}

// ============================================================================
// Helper trait for cleaner chaining
// ============================================================================

trait Pipe: Sized {
    fn pipe<F, T>(self, f: F) -> T
    where
        F: FnOnce(Self) -> T,
    {
        f(self)
    }
}

impl<T: Sized> Pipe for T {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_format_hash_and_verify() {
        let password = "my_secure_password_123";
        let hash = hash_password(password).unwrap();

        // New format starts with $argon2id
        assert!(hash.starts_with("$argon2id$"));

        // Verify succeeds
        let result = verify_password(password, &hash).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), hash); // no migration needed
    }

    #[test]
    fn test_verify_wrong_password() {
        let password = "correct_password";
        let hash = hash_password(password).unwrap();

        // 错误密码应返回 VerifyError
        let result = verify_password("wrong_password", &hash);
        assert!(
            matches!(result, Err(PasswordError::VerifyError)),
            "Wrong password should return VerifyError"
        );
    }

    #[test]
    fn test_legacy_plain_argon2_migration() {
        let password = "old_plain_pass";
        let argon2 = Argon2::default();
        let old_hash = argon2
            .hash_password(password.as_bytes())
            .unwrap()
            .to_string();
        // Argon2::default().hash_password() 已经生成标准 $argon2id$ 格式，不需要迁移
        assert!(
            !needs_migration(&old_hash),
            "Standard argon2id hash should not need migration"
        );

        // 验证应成功并返回原 hash（已经是新格式）
        let result = verify_password(password, &old_hash).unwrap();
        assert!(result.is_some());
        let new_hash = result.unwrap();
        assert!(new_hash.starts_with("$argon2id$"));
    }

    #[test]
    fn test_validate_password_policy() {
        assert!(
            matches!(
                validate_password_policy("short"),
                Err(PasswordPolicyError::TooShort { .. })
            ),
            "shorter than 8 should be rejected"
        );
        assert!(validate_password_policy("1234567").is_err());
        assert!(validate_password_policy("12345678").is_ok());
        assert!(validate_password_policy("a_very_long_password").is_ok());
    }
}
