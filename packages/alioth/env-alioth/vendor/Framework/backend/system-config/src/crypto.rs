//! Credentials Encryption
//!
//! 提供敏感凭证的加解密能力，使用 AES-256-GCM。
//! 加密密钥从环境变量 `SYSTEM_CONFIG_ENC_KEY` 读取，必须是 32 字节 Base64 编码字符串。

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::RngExt;
use std::sync::OnceLock;
use typenum::U12;
static CIPHER: OnceLock<Aes256Gcm> = OnceLock::new();

const NONCE_SIZE: usize = 12;

/// 初始化加密器（应在应用启动时调用一次）
pub fn init_encryption(key_b64: &str) -> Result<()> {
    let key_bytes = BASE64
        .decode(key_b64)
        .context("Invalid base64 encryption key")?;
    if key_bytes.len() != 32 {
        anyhow::bail!(
            "Encryption key must be 32 bytes after base64 decode, got {}",
            key_bytes.len()
        );
    }
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid AES-256-GCM key length: {:?}", e))?;
    let _ = CIPHER.set(cipher);
    Ok(())
}

/// 加密明文，返回 base64 编码的 `nonce:ciphertext`
pub fn encrypt(plaintext: &str) -> Result<String> {
    let cipher = CIPHER.get().context("Encryption not initialized")?;
    let nonce_bytes: [u8; 12] = rand::rng().random();
    #[allow(deprecated)]
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("Encrypt failed: {:?}", e))?;
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&combined))
}

/// 解密 base64 编码的 `nonce:ciphertext`
pub fn decrypt(ciphertext_b64: &str) -> Result<String> {
    let cipher = CIPHER.get().context("Encryption not initialized")?;
    let combined = BASE64
        .decode(ciphertext_b64)
        .context("Invalid base64 ciphertext")?;
    if combined.len() < NONCE_SIZE {
        anyhow::bail!("Ciphertext too short");
    }
    let nonce_bytes: [u8; 12] = combined[..NONCE_SIZE]
        .try_into()
        .map_err(|_| anyhow::anyhow!("Failed to extract 12-byte nonce"))?;
    let nonce = Nonce::<U12>::from(nonce_bytes);
    let plaintext = cipher
        .decrypt(&nonce, &combined[NONCE_SIZE..])
        .map_err(|e| anyhow::anyhow!("Decrypt failed: {:?}", e))?;
    String::from_utf8(plaintext).context("Invalid UTF-8 after decryption")
}

/// 解析带 `enc:` 前缀的凭证值：`enc:` ⇒ AES-256-GCM 解密；其余形态原样返回（兼容明文写入路径）。
///
/// 单一实现，供文件存储 / 邮箱 / LLM 等所有「DB 内 `enc_fields` 凭证」消费方共用。
/// 解密失败时 warn 并回退原值（不 panic）：调用方拿到的仍是原字符串（等价于未接解密器的旧行为），
/// 但日志点名根因（`SYSTEM_CONFIG_ENC_KEY` 未配置或与录入密钥不一致），避免静默把密文当口令发送。
pub fn decrypt_prefixed(value: &str) -> String {
    match value.strip_prefix("enc:") {
        Some(payload) => match decrypt(payload) {
            Ok(plain) => plain,
            Err(e) => {
                log::warn!(
                    "凭证解密失败（回退原值；检查 SYSTEM_CONFIG_ENC_KEY 是否配置且与录入时的密钥一致）: {e}"
                );
                value.to_string()
            }
        },
        None => value.to_string(),
    }
}

/// 对 JSON 对象中的指定 key 进行递归加密
pub fn encrypt_json_fields(value: &mut serde_json::Value, keys: &[&str]) -> Result<()> {
    if let Some(obj) = value.as_object_mut() {
        for (k, v) in obj.iter_mut() {
            if keys.contains(&k.as_str()) && v.is_string() {
                let plain = v.as_str().unwrap_or("");
                if !plain.is_empty() && !plain.starts_with("enc:") {
                    let encrypted = encrypt(plain)?;
                    *v = serde_json::Value::String(format!("enc:{}", encrypted));
                }
            }
        }
    }
    Ok(())
}

/// 对 JSON 对象中的指定 key 进行递归解密
pub fn decrypt_json_fields(value: &mut serde_json::Value, keys: &[&str]) -> Result<()> {
    if let Some(obj) = value.as_object_mut() {
        for (k, v) in obj.iter_mut() {
            if keys.contains(&k.as_str()) && v.is_string() {
                let encrypted = v.as_str().unwrap_or("");
                if let Some(payload) = encrypted.strip_prefix("enc:") {
                    let plain = decrypt(payload)?;
                    *v = serde_json::Value::String(plain);
                }
            }
        }
    }
    Ok(())
}

/// 生成一个新的 32 字节 Base64 编码密钥（用于初始化配置）
pub fn generate_key() -> String {
    let key: [u8; 32] = rand::random();
    BASE64.encode(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let key = generate_key();
        init_encryption(&key).unwrap();
        let plain = "my-secret-api-key-12345";
        let encrypted = encrypt(plain).unwrap();
        let decrypted = decrypt(&encrypted).unwrap();
        assert_eq!(plain, decrypted);
    }

    #[test]
    fn test_decrypt_prefixed() {
        let key = generate_key();
        init_encryption(&key).unwrap();

        // 明文（种子写入路径）：原样返回
        assert_eq!(decrypt_prefixed("plain-password"), "plain-password");

        // `enc:` 密文（system-config 写入路径）：解出原文
        let plain = "smtp-auth-code-42";
        let prefixed = format!("enc:{}", encrypt(plain).unwrap());
        assert_eq!(decrypt_prefixed(&prefixed), plain);

        // 坏密文：回退原值（不 panic——调用方据此走认证失败而非静默成功）
        let broken = "enc:not-valid-base64!!";
        assert_eq!(decrypt_prefixed(broken), broken);
    }

    #[test]
    fn test_json_fields_roundtrip() {
        let key = generate_key();
        init_encryption(&key).unwrap();
        let mut value = serde_json::json!({
            "api_key": "sk-123456",
            "base_url": "https://api.example.com",
            "timeout": 30
        });
        encrypt_json_fields(&mut value, &["api_key"]).unwrap();
        let api_key = value["api_key"].as_str().unwrap();
        assert!(api_key.starts_with("enc:"));

        decrypt_json_fields(&mut value, &["api_key"]).unwrap();
        assert_eq!(value["api_key"], "sk-123456");
        assert_eq!(value["base_url"], "https://api.example.com");
    }
}
