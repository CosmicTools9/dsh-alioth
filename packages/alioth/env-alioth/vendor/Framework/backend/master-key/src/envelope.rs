//! 密文信封：版本化（v2）与对旧三形态的级联解密。
//!
//! `v2` 载荷本体为 **裸** `v2:<key_id>:<base64(nonce‖ct‖tag)>`——`enc:` 前缀仍由调用层按各自
//! 载体约定添加（`.env` / `var_value` 加，`api_key_enc` 不加），从而两个消费 crate 的既有契约零变更。
//!
//! 形态判定（`enc:` 前缀可选；v2 优先识别）：
//!
//! | 形态 | 例 | 归属 |
//! | --- | --- | --- |
//! | [`Shape::V2`] | `v2:1a2b3c4d:<b64>` / `enc:v2:1a2b3c4d:<b64>` | 本 crate 产出 |
//! | [`Shape::GcmPrefixed`] | `enc:<b64>` | v1 DB 密文（GCM） |
//! | [`Shape::GcmRaw`] | `<b64>` | v1 DB 列裸密文（GCM） |
//! | [`Shape::ShellCbc`] | `enc:<hex_iv>:<b64>` | `.env` 文件（CBC，由 `env-vault.sh` 处理） |
//! | [`Shape::Plain`] | 其余 | 非密文 |
//!
//! Rust 侧 MUST NOT 解密 [`Shape::ShellCbc`]（算法与密钥使用方式属 shell 唯一实现）。

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::RngExt;

use crate::keyring::{Key, KeyError, Keyring, KEY_LEN};

const NONCE_SIZE: usize = 12;
const TAG_SIZE: usize = 16;
const V2_MARK: &str = "v2:";
const ENC_PREFIX: &str = "enc:";

/// 密文形态。
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Shape {
    /// `[enc:]v2:<key_id>:<base64(nonce‖ct‖tag)>`
    V2 { key_id: String, payload: String },
    /// `enc:<base64(nonce‖ct‖tag)>`（无第二冒号）
    GcmPrefixed(String),
    /// 裸 `<base64(nonce‖ct‖tag)>`
    GcmRaw(String),
    /// `enc:<hex_iv>:<base64>`——shell CBC，不由 Rust 解密
    ShellCbc,
    /// 非密文
    Plain,
}

/// 判定形态（纯函数）。
pub fn shape(stored: &str) -> Shape {
    let (had_enc, rest) = match stored.strip_prefix(ENC_PREFIX) {
        Some(r) => (true, r),
        None => (false, stored),
    };

    // v2 优先：无论是否带 `enc:` 前缀
    if let Some(v2) = rest.strip_prefix(V2_MARK) {
        return match v2.split_once(':') {
            Some((key_id, payload)) if !key_id.is_empty() && !payload.is_empty() => Shape::V2 {
                key_id: key_id.to_string(),
                payload: payload.to_string(),
            },
            _ => Shape::Plain,
        };
    }

    if had_enc {
        return if rest.contains(':') {
            Shape::ShellCbc
        } else if rest.is_empty() {
            Shape::Plain
        } else {
            Shape::GcmPrefixed(rest.to_string())
        };
    }

    if is_likely_base64(stored) {
        Shape::GcmRaw(stored.to_string())
    } else {
        Shape::Plain
    }
}

/// 是否为 shell CBC 形态（`enc:<iv>:<ct>`）。
pub fn is_shell_cbc(stored: &str) -> bool {
    shape(stored) == Shape::ShellCbc
}

/// 是否为「DB 侧密文」（v2 / GCM 前缀 / 裸 GCM）。
pub fn is_db_ciphertext(stored: &str) -> bool {
    matches!(
        shape(stored),
        Shape::V2 { .. } | Shape::GcmPrefixed(_) | Shape::GcmRaw(_)
    )
}

/// 明文 → 裸 `v2:<key_id>:<base64(nonce‖ct‖tag)>`。
///
/// 调用层按载体约定决定是否前置 `enc:`（`.env` / `var_value` 需要，`api_key_enc` 不需要）。
pub fn encode(key: &Key, plaintext: &str) -> Result<String, KeyError> {
    let payload = gcm_encrypt(key.raw(), plaintext.as_bytes())?;
    Ok(format!("{V2_MARK}{}:{payload}", key.id()))
}

/// 密文 → 明文。级联：v2 按 `key_id` 选钥；旧形态遍历 keyring 全部钥。
///
/// 缺钥 / 形态不符 / 解密失败均 **fail-loud**，MUST NOT 回退原字符串。
pub fn decode_with(ring: &Keyring, stored: &str) -> Result<String, KeyError> {
    match shape(stored) {
        Shape::V2 { key_id, payload } => {
            let key = ring
                .find(&key_id)
                .ok_or_else(|| KeyError::UnknownKeyId(key_id.clone()))?;
            gcm_decrypt_to_string(key.raw(), &payload)
        }
        Shape::GcmPrefixed(payload) => try_all(ring, &payload),
        Shape::GcmRaw(payload) => try_all(ring, &payload),
        Shape::ShellCbc => Err(KeyError::UnsupportedForm(stored.chars().take(24).collect())),
        Shape::Plain => Err(KeyError::Malformed(stored.chars().take(24).collect())),
    }
}

/// 遍历 keyring 全部钥尝试解密（供无 `key_id` 的旧形态使用）。
fn try_all(ring: &Keyring, payload: &str) -> Result<String, KeyError> {
    let mut attempts = 0usize;
    let mut last = String::new();
    for key in ring.iter() {
        attempts += 1;
        match gcm_decrypt_to_string(key.raw(), payload) {
            Ok(plain) => return Ok(plain),
            Err(e) => last = e.to_string(),
        }
    }
    Err(KeyError::Decrypt(attempts, last))
}

/// AES-256-GCM 加密 → `base64(nonce‖ct‖tag)`。
fn gcm_encrypt(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<String, KeyError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| KeyError::Cipher(format!("AES 初始化失败: {e:?}")))?;
    let nonce_bytes: [u8; NONCE_SIZE] = rand::rng().random();
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| KeyError::Cipher(format!("加密失败: {e:?}")))?;
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&combined))
}

/// `base64(nonce‖ct‖tag)` → 明文。
fn gcm_decrypt_to_string(key: &[u8; KEY_LEN], payload_b64: &str) -> Result<String, KeyError> {
    let combined = BASE64
        .decode(payload_b64)
        .map_err(|e| KeyError::Cipher(format!("密文非合法 base64: {e}")))?;
    if combined.len() < NONCE_SIZE + TAG_SIZE {
        return Err(KeyError::Cipher(format!(
            "密文过短（{} 字节，至少需 {}）",
            combined.len(),
            NONCE_SIZE + TAG_SIZE
        )));
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| KeyError::Cipher(format!("AES 初始化失败: {e:?}")))?;
    let nonce_bytes: [u8; NONCE_SIZE] = combined[..NONCE_SIZE]
        .try_into()
        .map_err(|_| KeyError::Cipher("nonce 提取失败".to_string()))?;
    let nonce = Nonce::from(nonce_bytes);
    let plaintext = cipher
        .decrypt(&nonce, &combined[NONCE_SIZE..])
        .map_err(|e| KeyError::Cipher(format!("解密认证失败: {e:?}")))?;
    String::from_utf8(plaintext).map_err(|e| KeyError::Cipher(format!("明文非 UTF-8: {e}")))
}

/// 粗判 base64 字母表（用于区分裸密文与明文；纯函数、不解析语义）。
fn is_likely_base64(s: &str) -> bool {
    !s.is_empty()
        && s.len() >= 24
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> Key {
        Key::from_raw([seed; KEY_LEN])
    }

    /// 从 v2 载荷抽出 `(key_id, payload)`，供构造旧形态样本。
    fn split_v2(s: &str) -> (String, String) {
        let body = s.strip_prefix(V2_MARK).expect("v2 前缀");
        let (kid, payload) = body.split_once(':').expect("v2 结构");
        (kid.to_string(), payload.to_string())
    }

    #[test]
    fn v2_roundtrip_carries_key_id() {
        let k = key(7);
        let enc = encode(&k, "hello-secret").expect("encrypt");
        assert!(enc.starts_with("v2:"), "新载荷 MUST 带版本标记: {enc}");
        let s = shape(&enc);
        assert!(
            matches!(&s, Shape::V2 { key_id, .. } if key_id == k.id()),
            "key_id MUST 等于加密钥标识"
        );
        let ring = Keyring::new(k.clone(), vec![]);
        assert_eq!(decode_with(&ring, &enc).expect("decrypt"), "hello-secret");
    }

    #[test]
    fn v2_with_enc_prefix_also_recognized() {
        let k = key(7);
        let enc = format!("enc:{}", encode(&k, "prefixed").expect("encrypt"));
        let ring = Keyring::new(k, vec![]);
        assert_eq!(decode_with(&ring, &enc).expect("decrypt"), "prefixed");
    }

    #[test]
    fn v1_gcm_prefixed_still_decodes() {
        let k = key(3);
        let legacy = format!("enc:{}", gcm_encrypt(k.raw(), b"legacy").unwrap());
        let ring = Keyring::new(k, vec![]);
        assert_eq!(decode_with(&ring, &legacy).expect("decrypt"), "legacy");
    }

    #[test]
    fn v1_gcm_raw_still_decodes() {
        let k = key(4);
        let legacy = gcm_encrypt(k.raw(), b"raw-form").unwrap();
        let ring = Keyring::new(k, vec![]);
        assert_eq!(decode_with(&ring, &legacy).expect("decrypt"), "raw-form");
    }

    #[test]
    fn previous_key_decodes_old_ciphertext() {
        let old = key(1);
        let new = key(2);
        let enc = format!("enc:{}", gcm_encrypt(old.raw(), b"pre-rotation").unwrap());
        let ring = Keyring::new(new, vec![old]);
        assert_eq!(
            decode_with(&ring, &enc).expect("previous 钥 MUST 可解旧密文"),
            "pre-rotation"
        );
    }

    #[test]
    fn v2_unknown_key_id_fails_loud() {
        let (_, payload) = split_v2(&encode(&key(9), "x").expect("encrypt"));
        let enc = format!("v2:deadbeef:{payload}");
        let ring = Keyring::new(key(2), vec![]);
        let err = decode_with(&ring, &enc).expect_err("缺钥 MUST 报错");
        assert!(
            matches!(&err, KeyError::UnknownKeyId(id) if id == "deadbeef"),
            "错误 MUST 点名缺失的 key_id，实得 {err:?}"
        );
    }

    #[test]
    fn shell_cbc_is_rejected_by_rust() {
        let ring = Keyring::new(key(1), vec![]);
        let err = decode_with(&ring, "enc:00112233445566778899aabbccddeeff:AAAA")
            .expect_err("CBC 不由 Rust 解");
        assert!(matches!(err, KeyError::UnsupportedForm(_)));
    }

    #[test]
    fn wrong_key_reports_attempt_count() {
        let enc = format!("enc:{}", gcm_encrypt(key(1).raw(), b"nope").unwrap());
        let ring = Keyring::new(key(2), vec![key(3)]);
        match decode_with(&ring, &enc) {
            Err(KeyError::Decrypt(n, _)) => assert_eq!(n, 2, "MUST 遍历 active + previous 两把钥"),
            other => panic!("应为 Decrypt 错误，实得 {other:?}"),
        }
    }

    #[test]
    fn shape_classifies_plain_and_cbc() {
        assert_eq!(shape("just-a-value"), Shape::Plain);
        assert_eq!(
            shape("enc:00112233445566778899aabbccddeeff:AAAA"),
            Shape::ShellCbc
        );
        assert!(is_shell_cbc("enc:aabb:ccdd"));
        assert!(!is_shell_cbc("enc:dGVzdA=="));
        assert!(is_db_ciphertext("v2:abcd1234:AAAA"));
    }
}
