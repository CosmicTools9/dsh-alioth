//! Credentials Encryption
//!
//! 提供敏感凭证的加解密能力，使用 AES-256-GCM。
//! 加密密钥从环境变量 `SYSTEM_CONFIG_ENC_KEY` 读取，必须是 32 字节 Base64 编码字符串。

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use master_key::{MasterKey, ProvidedKeyLoader};
use std::sync::OnceLock;

/// 进程级主密钥环。
///
/// active = 调用方传入的 `SYSTEM_CONFIG_ENC_KEY`；previous 来自环境变量
/// `SYSTEM_CONFIG_ENC_KEY_PREVIOUS`（逗号分隔）与目录 `SYSTEM_CONFIG_ENC_KEYRING_DIR`（可选）。
/// 轮换过渡期旧钥留在环内 ⇒ 旧密文全程可读 ⇒ 无停机窗口。
static MASTER_KEY: OnceLock<MasterKey> = OnceLock::new();

/// 初始化加密器（应在应用启动时调用一次）
pub fn init_encryption(key_b64: &str) -> Result<()> {
    let mut loader =
        ProvidedKeyLoader::new(key_b64).with_previous_env("SYSTEM_CONFIG_ENC_KEY_PREVIOUS");
    if let Ok(dir) = std::env::var("SYSTEM_CONFIG_ENC_KEYRING_DIR") {
        if !dir.trim().is_empty() {
            loader = loader.with_previous_dir(dir);
        }
    }
    let master =
        MasterKey::load(Box::new(loader)).context("Failed to load system-config keyring")?;
    let _ = MASTER_KEY.set(master);
    Ok(())
}

fn master_key() -> Result<&'static MasterKey> {
    MASTER_KEY.get().context("Encryption not initialized")
}

/// 加密明文 → `v2:<key_id>:<base64(nonce‖ct‖tag)>`（裸载荷；调用层按需前置 `enc:`）
pub fn encrypt(plaintext: &str) -> Result<String> {
    master_key()?
        .encrypt(plaintext)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// 解密 → 明文。级联兼容 `v2:<kid>:<b64>` 与既有 v1 形态（`enc:<b64>` / 裸 `<b64>`）。
pub fn decrypt(ciphertext: &str) -> Result<String> {
    master_key()?
        .decrypt(ciphertext)
        .map_err(|e| anyhow::anyhow!("{e}"))
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
