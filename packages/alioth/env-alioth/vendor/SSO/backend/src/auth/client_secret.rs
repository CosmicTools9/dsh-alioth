//! OIDC client_secret 散列与校验
//!
//! 新标准：argon2id（`$argon2id$…`），与 `password.rs` 一致。
//! 遗留格式：MD5 十六进制（32 字符），由旧版 `oidc_clients` 写入
//! （DDL 注释虽写 SHA-256，但历史实现实际为 MD5）。
//!
//! 校验同时支持两种格式：argon2id 为权威格式；MD5 在验证成功时返回
//! 新算出的 argon2id 散列，便于调用方在轮转 secret 时完成迁移。
//!
//! 注意：MD5 为单向散列且明文 secret 仅在创建时返回一次，无法对存量记录
//! 批量重算，因此采用「创建 / 轮转时迁移」策略——新 secret 一律存 argon2id，
//! 存量 MD5 记录在读时仍可验证，直至管理员轮转 secret 自然升级。

use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use md5;
use thiserror::Error;

/// client_secret 散列/校验错误
#[derive(Debug, Error)]
pub enum ClientSecretError {
    #[error("Failed to hash client_secret")]
    HashError,
    #[error("Failed to verify client_secret")]
    VerifyError,
    #[error("Invalid hash format")]
    InvalidHashFormat,
}

/// MD5 十六进制长度
const MD5_HEX_LEN: usize = 32;

/// 以 argon2id 散列 client_secret，返回以 `$argon2id$` 开头的字符串。
pub fn hash_client_secret(secret: &str) -> Result<String, ClientSecretError> {
    Argon2::default()
        .hash_password(secret.as_bytes())
        .map_err(|_| ClientSecretError::HashError)?
        .to_string()
        .pipe(Ok)
}

/// 校验 client_secret 是否匹配已存储散列。
///
/// 返回：
/// - `Ok(Some(hash))`：匹配成功，`hash` 为该 secret 的规范散列字符串。
///   若输入为遗留 MD5 格式，返回值为新算出的 argon2id 散列，便于透明迁移。
/// - `Err(_)`：不匹配或散列格式损坏（与 `password.rs::verify_password` 语义一致，
///   不存在 `Ok(None)` 分支）。
pub fn verify_client_secret(
    secret: &str,
    stored: &str,
) -> Result<Option<String>, ClientSecretError> {
    // 1. 新标准 argon2id
    if stored.starts_with("$argon2") {
        let parsed = PasswordHash::new(stored).map_err(|_| ClientSecretError::InvalidHashFormat)?;
        Argon2::default()
            .verify_password(secret.as_bytes(), &parsed)
            .map_err(|_| ClientSecretError::VerifyError)?;
        return Ok(Some(stored.to_string()));
    }

    // 2. 遗留 MD5（32 位十六进制）
    if stored.len() == MD5_HEX_LEN && stored.chars().all(|c| c.is_ascii_hexdigit()) {
        let expected = format!("{:x}", md5::compute(secret.as_bytes()));
        if expected.eq_ignore_ascii_case(stored) {
            return Ok(Some(hash_client_secret(secret)?));
        }
        return Err(ClientSecretError::VerifyError);
    }

    Err(ClientSecretError::InvalidHashFormat)
}

/// 若散列非新标准 argon2id 格式（即 MD5 遗留），返回 true。
pub fn needs_migration(stored: &str) -> bool {
    !stored.starts_with("$argon2")
}

// ============================================================================
// 异步包装：Argon2 是 CPU/内存密集型操作，必须在 blocking pool 执行，
// 否则会阻塞 tokio worker，导致 SSO 在并发请求下挂起超时。
// ============================================================================

/// 异步版本 hash_client_secret，内部在 spawn_blocking 中执行 Argon2。
pub async fn hash_client_secret_async(secret: String) -> Result<String, ClientSecretError> {
    tokio::task::spawn_blocking(move || hash_client_secret(&secret))
        .await
        .map_err(|_| ClientSecretError::HashError)?
}

/// 异步版本 verify_client_secret，内部在 spawn_blocking 中执行 Argon2。
pub async fn verify_client_secret_async(
    secret: String,
    stored: String,
) -> Result<Option<String>, ClientSecretError> {
    tokio::task::spawn_blocking(move || verify_client_secret(&secret, &stored))
        .await
        .map_err(|_| ClientSecretError::VerifyError)?
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
    fn argon2id_hash_and_verify_ok() {
        let h = hash_client_secret("topsecret").unwrap();
        assert!(h.starts_with("$argon2id$"));
        let r = verify_client_secret("topsecret", &h).unwrap();
        assert_eq!(r.unwrap(), h);
    }

    #[test]
    fn argon2id_wrong_secret_fails() {
        let h = hash_client_secret("topsecret").unwrap();
        assert!(matches!(
            verify_client_secret("nope", &h),
            Err(ClientSecretError::VerifyError)
        ));
    }

    #[test]
    fn md5_legacy_verify_and_migrate() {
        let md5_hash = format!("{:x}", md5::compute(b"legacypass"));
        let r = verify_client_secret("legacypass", &md5_hash).unwrap();
        let migrated = r.expect("legacy secret should verify");
        assert!(migrated.starts_with("$argon2id$"));
        // 迁移后的 argon2id 散列可再次验证
        assert!(verify_client_secret("legacypass", &migrated)
            .unwrap()
            .is_some());
    }

    #[test]
    fn md5_legacy_wrong_secret_fails() {
        let md5_hash = format!("{:x}", md5::compute(b"legacypass"));
        assert!(matches!(
            verify_client_secret("wrong", &md5_hash),
            Err(ClientSecretError::VerifyError)
        ));
    }

    #[test]
    fn invalid_format_errors() {
        assert!(matches!(
            verify_client_secret("x", "not-a-hash"),
            Err(ClientSecretError::InvalidHashFormat)
        ));
    }
}
