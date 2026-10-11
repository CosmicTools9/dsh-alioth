//! PEP 内部 seam — JWT 解码/验签（评审候选 5 分缝之一：jwt）。
//!
//! 自 `pep/middleware.rs` 内部抽出；外 interface 不变——本模块仅供
//! `pep::middleware`（`NgacEnforcer` 内部）消费，集成测试经 `NgacEnforcer` 间接覆盖。

use super::jwks::{JwksError, SsoJwksClient};
use super::pdp_client::PdpError;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

/// 测试用 EC P-256 公钥（PKCS#8 PEM），供 `new_without_pool()` 与集成测试使用。
pub(super) const TEST_SSO_JWT_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAExHBYWD4VZBXSBjQIgcMUKbtHGEV3
\
NK6CQd0RxdS3yLGgsXJ1XqdLJwuPvErVIsSI3ywGfDHPPrqmuN53XjRBZg==
\
-----END PUBLIC KEY-----";

/// JWT Claims structure for authentication.
///
/// Extracted from the Authorization header's Bearer token.
/// Contains user identity and token metadata.
///
/// # Fields
///
/// * `sub` - Subject (user ID) from JWT
/// * `exp` - Expiration timestamp (Unix seconds)
/// * `iat` - Issued-at timestamp (Unix seconds)
/// * `email` - User email address (optional, for audit logging)
///
/// # Example
///
/// ```json
/// {
///   "sub": "550e8400-e29b-41d4-a716-446655440000",
///   "exp": 1234567890,
///   "iat": 1234567800,
///   "email": "user@example.com"
/// }
/// ```
#[derive(Debug, Deserialize, Clone)]
pub struct Claims {
    /// Subject (user ID) from JWT
    pub sub: String,
    /// Expiration time (Unix timestamp in seconds)
    pub exp: usize,
    /// Issued at (Unix timestamp in seconds)
    pub iat: usize,
    /// User email address for audit logging (defaults to empty string)
    #[serde(default)]
    pub email: String,
    /// Username for display
    #[serde(default)]
    pub username: String,
    /// Session token linking this token to an SSO session.
    /// When present, the PEP validates session revocation status.
    #[serde(default)]
    pub sid: String,
    /// Issuer（必须与 SSO `oidc_issuer` 一致），用于防令牌错投。
    #[serde(default)]
    pub iss: String,
    /// Audience（第一方场景 == issuer），验证方必须校验命中自身预期值。
    #[serde(default)]
    pub aud: String,
    /// OAuth scope（空格分隔）；client_credentials 服务令牌携带其子集。
    #[serde(default)]
    pub scope: String,
    /// 服务令牌主体：api_clients.fk_service_user（auth_users 服务用户 id）。
    /// 自然人令牌为 0。服务令牌以此作为 NGAC PDP 决策主体（openapi-external-access）。
    #[serde(default)]
    #[serde(with = "common::serde_zuid")]
    pub svc_user_id: i64,
    /// standalone token_version（fix-sso-noauth-removal）：standalone 登录 token
    /// 携带（Some）；SSO/服务令牌无此声明（None）。PEP 对 Some 值比对
    /// `standalone_users.token_version`——logout bump 后旧 token 即时失效。
    #[serde(default)]
    pub tv: Option<i64>,
}

/// Decode JWT token and extract claims.
///
/// 优先通过 SSO JWKS（按 token header `kid` 选密钥）验证；JWKS 不可用或缺失对应
/// 密钥时，回退到静态配置的公钥（若存在）。`jwks_client` 为 None 时直接使用静态公钥。
/// 除 `exp` 外，强制校验 `iss` 与 `aud`：jsonwebtoken 10 的 `set_issuer`/`set_audience`
/// 为条件校验（claim 缺失即跳过），故 decode 成功后显式比对——`iss`/`aud` 必须存在
/// 且等于预期绑定值，缺失或不匹配 → 401（防令牌跨部署/服务错投被接受）。
pub(super) async fn decode_jwt_token(
    jwks_client: Option<&SsoJwksClient>,
    fallback_keys: &[&[u8]],
    token: &str,
    issuer: &str,
    audience: &str,
) -> Result<Claims, PdpError> {
    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);

    let decoding_key = match jwks_client {
        Some(client) => {
            let kid = SsoJwksClient::token_kid(token);
            match client.decoding_key(&kid).await {
                Ok(key) => Some(key),
                // 仅 SSO 不可用（网络/HTTP/解析失败）回退静态公钥（可用性）；
                // kid 未命中 / 密钥不支持是吊销或配置信号，fail-closed 拒绝
                // （M69 审计 P3：回退会掩盖已吊销 key 的令牌）。
                Err(JwksError::FetchFailed(e)) => {
                    log::debug!("JWKS 不可用，回退静态公钥: {e}");
                    None
                }
                Err(e) => {
                    return Err(PdpError::TokenDecodeError(format!(
                        "JWKS 验证失败（fail-closed）: {e}"
                    )))
                }
            }
        }
        None => None,
    };

    let claims = match decoding_key {
        // JWKS 命中 → 单钥验证（kid 已选钥）
        Some(key) => {
            decode::<Claims>(token, &key, &validation)
                .map_err(|e| PdpError::TokenDecodeError(e.to_string()))?
                .claims
        }
        // 无 JWKS / JWKS 不可用 → 静态回退（active + prev 依次尝试，轮换窗口旧钥可验）
        None => verify_token_with_keys(fallback_keys, token, &validation)?,
    };

    // 显式强制 iss/aud（jsonwebtoken 10 的 set_* 仅条件校验）：声明缺失或不匹配
    // → 视为无效令牌（401），杜绝「缺 aud/iss 声明跳过绑定校验」的错投路径。
    if claims.iss.is_empty() || claims.iss != issuer {
        return Err(PdpError::TokenDecodeError(format!(
            "iss claim missing or mismatch: expected '{}'",
            issuer
        )));
    }
    if claims.aud.is_empty() || claims.aud != audience {
        return Err(PdpError::TokenDecodeError(format!(
            "aud claim missing or mismatch: expected '{}'",
            audience
        )));
    }

    Ok(claims)
}

/// 静态回退多 key 验签：active + prev 依次尝试，任一把成功即接受；
/// 全部失败返回第一个错误（保持单钥错误语义），空列表返回明确错误。
pub(super) fn verify_token_with_keys(
    keys: &[&[u8]],
    token: &str,
    validation: &Validation,
) -> Result<Claims, PdpError> {
    let mut first_err: Option<PdpError> = None;
    for key in keys {
        let decoding_key = match DecodingKey::from_ec_pem(key) {
            Ok(dk) => dk,
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(PdpError::TokenDecodeError(e.to_string()));
                }
                continue;
            }
        };
        match decode::<Claims>(token, &decoding_key, validation) {
            Ok(td) => return Ok(td.claims),
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(PdpError::TokenDecodeError(e.to_string()));
                }
            }
        }
    }
    Err(first_err.unwrap_or_else(|| {
        PdpError::TokenDecodeError("no static fallback public key configured".to_string())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claims_deserialize() {
        let json = r#"{
            "sub": "550e8400-e29b-41d4-a716-446655440000",
            "exp": 1234567890,
            "iat": 1234567800
        }"#;

        let claims: Claims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.sub, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(claims.exp, 1234567890);
        assert_eq!(claims.iat, 1234567800);
    }

    #[test]
    fn test_verify_token_with_keys_no_keys() {
        let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        let err = verify_token_with_keys(&[], "not-a-token", &validation);
        assert!(matches!(err, Err(PdpError::TokenDecodeError(_))));
    }

    #[test]
    fn test_verify_token_with_keys_bad_pem() {
        let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        let bad = b"-----BEGIN PUBLIC KEY-----\nnot-a-key\n-----END PUBLIC KEY-----";
        let err = verify_token_with_keys(&[bad], "not-a-token", &validation);
        assert!(matches!(err, Err(PdpError::TokenDecodeError(_))));
    }

    #[test]
    fn test_verify_token_with_keys_second_key_used() {
        // 第一把钥解析失败但第二把是合法公钥：验证不因第一把坏钥短路（走第二把）
        let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        let bad = b"-----BEGIN PUBLIC KEY-----\nnot-a-key\n-----END PUBLIC KEY-----";
        let err =
            verify_token_with_keys(&[bad, TEST_SSO_JWT_PUBLIC_KEY], "not-a-token", &validation);
        // token 无效（非坏钥导致），错误应为 decode 类而非坏钥短路
        assert!(matches!(err, Err(PdpError::TokenDecodeError(_))));
    }
}
