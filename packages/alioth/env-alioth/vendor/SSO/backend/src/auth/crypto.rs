//! 敏感字段静态加密（AES-256-GCM）
//!
//! 用于 MFA 共享密钥等落库敏感数据。密钥由 `ENCRYPTION_KEY`（Config.encryption_key）
//! 经 SHA-256 派生为 32 字节，配合每次随机 12 字节 nonce（避免确定性 nonce 复用问题）。
//! 密文格式：`nonce(12) || ciphertext`。

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use sha2::{Digest, Sha256};

const NONCE_LEN: usize = 12;

/// 由 `ENCRYPTION_KEY` 派生 32 字节 AES-256 密钥（确定性，便于轮换 `ENCRYPTION_KEY`）。
fn derive_key(encryption_key: &[u8]) -> [u8; 32] {
    let out = Sha256::digest(encryption_key);
    <[u8; 32]>::try_from(out.as_slice()).expect("SHA-256 output is exactly 32 bytes")
}

/// 加密明文，返回 `nonce || ciphertext`。
pub fn encrypt_secret(encryption_key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(&derive_key(encryption_key))
        .map_err(|e| format!("cipher init failed: {e}"))?;
    let nonce: [u8; NONCE_LEN] = rand::random();
    let nonce =
        Nonce::try_from(nonce.as_slice()).map_err(|_| "invalid nonce length".to_string())?;
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("encrypt failed: {e}"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// 解密 `nonce || ciphertext`，还原明文。
pub fn decrypt_secret(encryption_key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < NONCE_LEN {
        return Err("ciphertext too short".to_string());
    }
    let (nonce, ciphertext) = data.split_at(NONCE_LEN);
    let nonce = Nonce::try_from(nonce).map_err(|_| "invalid nonce length".to_string())?;
    let cipher = Aes256Gcm::new_from_slice(&derive_key(encryption_key))
        .map_err(|e| format!("cipher init failed: {e}"))?;
    cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|e| format!("decrypt failed: {e}"))
}

/// 将加密结果编码为可存库的字符串（`enc:` 前缀区分旧明文 base32）。
pub fn encode_encrypted(encryption_key: &[u8], plaintext: &[u8]) -> Result<String, String> {
    let raw = encrypt_secret(encryption_key, plaintext)?;
    Ok(format!("enc:{}", BASE64.encode(raw)))
}

/// 从存库字符串还原明文：支持 `enc:` 前缀密文与旧 base32 明文（向后兼容迁移）。
pub fn decode_secret(encryption_key: &[u8], stored: &str) -> Result<String, String> {
    if let Some(b64) = stored.strip_prefix("enc:") {
        let raw = BASE64
            .decode(b64)
            .map_err(|e| format!("base64 decode failed: {e}"))?;
        let pt = decrypt_secret(encryption_key, &raw)?;
        String::from_utf8(pt).map_err(|e| format!("utf8 decode failed: {e}"))
    } else {
        // 旧格式：直接是 base32 字符串（未加密），原样返回以兼容存量数据。
        Ok(stored.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = b"test-encryption-key";
        let secret = "JBSWY3DPEHPK3PXP"; // 典型 TOTP base32 secret
        let stored = encode_encrypted(key, secret.as_bytes()).unwrap();
        assert!(stored.starts_with("enc:"));
        let back = decode_secret(key, &stored).unwrap();
        assert_eq!(back, secret);
    }

    #[test]
    fn test_decode_legacy_plaintext_compat() {
        let key = b"test-encryption-key";
        let legacy = "JBSWY3DPEHPK3PXP";
        // 旧明文无 enc: 前缀，应原样返回（迁移期兼容）
        assert_eq!(decode_secret(key, legacy).unwrap(), legacy);
    }

    #[test]
    fn test_unique_nonce_per_encryption() {
        let key = b"test-encryption-key";
        let a = encode_encrypted(key, b"same").unwrap();
        let b = encode_encrypted(key, b"same").unwrap();
        // 随机 nonce 保证相同明文密文不同
        assert_ne!(a, b);
    }

    #[test]
    fn test_tampered_ciphertext_rejected() {
        let key = b"test-encryption-key";
        let stored = encode_encrypted(key, b"secret-value").unwrap();
        // 取出密文 base64 体并翻转末字节，破坏 GCM 认证标签
        let b64 = stored.strip_prefix("enc:").expect("expected enc: prefix");
        let mut raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("base64 decode");
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        let tampered = format!(
            "enc:{}",
            base64::engine::general_purpose::STANDARD.encode(&raw)
        );
        assert!(
            decode_secret(key, &tampered).is_err(),
            "篡改后的密文必须被 GCM 认证拒绝"
        );
    }
}
