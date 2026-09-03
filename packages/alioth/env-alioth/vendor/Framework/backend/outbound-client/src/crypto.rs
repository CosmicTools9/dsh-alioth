//! 出向调用方凭据加解密（AES-256-GCM，enc: 前缀，格式对齐 system_config::crypto）。
//!
//! 密钥 = `OUTBOUND_ENC_KEY`（32 字节 base64）；兼容旧名 `WZ_FSSC_ENC_KEY`。
//! 密文格式：`enc:<nonce_b64>:<ciphertext_b64>`。

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::TryRng;

use common::AliothError as ApiError;

const ENC_PREFIX: &str = "enc:";
const NONCE_SIZE: usize = 12;

/// 读取加密密钥（OUTBOUND_ENC_KEY，32 字节 base64；兼容 WZ_FSSC_ENC_KEY）
fn key_bytes() -> Result<[u8; 32], ApiError> {
    let raw = std::env::var("OUTBOUND_ENC_KEY")
        .or_else(|_| std::env::var("WZ_FSSC_ENC_KEY"))
        .map_err(|_| ApiError::Internal("OUTBOUND_ENC_KEY 未设置：无法解密出向凭据".to_string()))?;
    let decoded = BASE64
        .decode(raw.trim())
        .map_err(|e| ApiError::Internal(format!("OUTBOUND_ENC_KEY 非合法 base64: {}", e)))?;
    let arr: [u8; 32] = decoded
        .try_into()
        .map_err(|_| ApiError::Internal("OUTBOUND_ENC_KEY 必须为 32 字节".to_string()))?;
    Ok(arr)
}

/// 加密明文 → `enc:<nonce_b64>:<ciphertext_b64>`
pub fn encrypt(plaintext: &str) -> Result<String, ApiError> {
    let key = key_bytes()?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| ApiError::Internal(format!("AES key 无效: {:?}", e)))?;
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::rng()
        .try_fill_bytes(&mut nonce_bytes)
        .map_err(|e| ApiError::Internal(format!("nonce 生成失败: {}", e)))?;
    let nonce: aes_gcm::aead::Nonce<Aes256Gcm> =
        aes_gcm::aead::Nonce::<Aes256Gcm>::try_from(&nonce_bytes[..])
            .map_err(|_| ApiError::Internal("nonce 长度无效".to_string()))?;
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| ApiError::Internal(format!("加密失败: {:?}", e)))?;
    Ok(format!(
        "{}{}:{}",
        ENC_PREFIX,
        BASE64.encode(nonce_bytes),
        BASE64.encode(ct)
    ))
}

/// 解密 `enc:...` → 明文；非 enc: 前缀原样返回（兼容明文配置）
pub fn decrypt(ciphertext: &str) -> Result<String, ApiError> {
    if !ciphertext.starts_with(ENC_PREFIX) {
        return Ok(ciphertext.to_string());
    }
    let body = &ciphertext[ENC_PREFIX.len()..];
    let (nonce_b64, ct_b64) = body
        .split_once(':')
        .ok_or_else(|| ApiError::Internal("enc: 密文格式错误（缺 :）".to_string()))?;
    let nonce_bytes = BASE64
        .decode(nonce_b64)
        .map_err(|e| ApiError::Internal(format!("enc: nonce 解码失败: {}", e)))?;
    let ct = BASE64
        .decode(ct_b64)
        .map_err(|e| ApiError::Internal(format!("enc: 密文解码失败: {}", e)))?;
    let key = key_bytes()?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| ApiError::Internal(format!("AES key 无效: {:?}", e)))?;
    let nonce: aes_gcm::aead::Nonce<Aes256Gcm> =
        aes_gcm::aead::Nonce::<Aes256Gcm>::try_from(&nonce_bytes[..])
            .map_err(|_| ApiError::Internal("nonce 长度无效".to_string()))?;
    let pt = cipher
        .decrypt(&nonce, ct.as_ref())
        .map_err(|e| ApiError::Internal(format!("解密失败: {:?}", e)))?;
    String::from_utf8(pt).map_err(|e| ApiError::Internal(format!("明文非 UTF-8: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypto_roundtrip_and_plain() {
        std::env::set_var(
            "OUTBOUND_ENC_KEY",
            "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=",
        );
        let plain = "s3cret";
        let enc = encrypt(plain).expect("encrypt");
        assert!(enc.starts_with("enc:"));
        assert_eq!(decrypt(&enc).unwrap(), plain);
        assert_eq!(decrypt("plain-secret").unwrap(), "plain-secret");
        std::env::remove_var("OUTBOUND_ENC_KEY");
    }
}
