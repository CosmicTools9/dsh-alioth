//! OIDC (OpenID Connect) 协议实现
//!
//! 提供 OIDC ID Token 验证、用户信息获取等功能：
//! - ID Token 解析和验证
//! - JWKS 密钥获取和缓存
//! - 用户信息标准化

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, TokenData, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// OIDC ID Token Claims
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IdTokenClaims {
    /// Subject - 用户的唯一标识符 (由身份提供商分配)
    pub sub: String,
    /// Issuer - 签发者
    pub iss: String,
    /// Audience - 受众 (客户端 ID)
    pub aud: String,
    /// Expiration Time - 过期时间
    #[serde(with = "common::serde_zuid")]
    pub exp: i64,
    /// Issued At - 签发时间
    #[serde(with = "common::serde_zuid")]
    pub iat: i64,
    /// 可选：用户邮箱
    pub email: Option<String>,
    /// 可选：邮箱是否已验证
    pub email_verified: Option<bool>,
    /// 可选：用户全名
    pub name: Option<String>,
    /// 可选：用户昵称
    pub nickname: Option<String>,
    /// 可选：用户头像 URL
    pub picture: Option<String>,
    /// 可选：给定名
    pub given_name: Option<String>,
    /// 可选：姓氏
    pub family_name: Option<String>,
    /// 可选：首选用户名
    pub preferred_username: Option<String>,
    /// 可选：授权时间
    #[serde(with = "common::serde_zuid::opt", default)]
    pub auth_time: Option<i64>,
    /// 可选：随机数
    pub nonce: Option<String>,
    /// 其他声明
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// JWKS (JSON Web Key Set) 响应
#[derive(Debug, Deserialize, Clone)]
pub struct JwksResponse {
    pub keys: Vec<Jwk>,
}

/// JSON Web Key
#[derive(Debug, Deserialize, Clone)]
pub struct Jwk {
    /// Key Type
    pub kty: String,
    /// Key ID
    pub kid: Option<String>,
    /// Algorithm
    pub alg: Option<String>,
    /// Usage
    pub r#use: Option<String>,
    /// RSA Modulus (for RSA keys)
    pub n: Option<String>,
    /// RSA Exponent (for RSA keys)
    pub e: Option<String>,
    /// X Coordinate (for EC keys)
    pub x: Option<String>,
    /// Y Coordinate (for EC keys)
    pub y: Option<String>,
    /// Curve (for EC keys)
    pub crv: Option<String>,
}

/// OIDC 发现文档 (OpenID Provider Metadata)
#[derive(Debug, Deserialize, Clone)]
pub struct OidcDiscoveryDocument {
    /// Issuer
    pub issuer: String,
    /// Authorization Endpoint
    pub authorization_endpoint: String,
    /// Token Endpoint
    pub token_endpoint: String,
    /// UserInfo Endpoint
    pub userinfo_endpoint: Option<String>,
    /// JWKS URI
    pub jwks_uri: String,
    /// Supported Scopes
    pub scopes_supported: Option<Vec<String>>,
    /// Supported Response Types
    pub response_types_supported: Vec<String>,
    /// Supported Grant Types
    pub grant_types_supported: Option<Vec<String>>,
    /// Supported Subject Types
    pub subject_types_supported: Vec<String>,
    /// Supported ID Token Signing Algorithms
    pub id_token_signing_alg_values_supported: Vec<String>,
}

/// OIDC 客户端
pub struct OidcClient {
    client_id: String,
    issuer: String,
    jwks_uri: String,
    jwks_cache: Option<(JwksResponse, std::time::Instant)>,
}

impl OidcClient {
    /// 创建新的 OIDC 客户端
    pub fn new(
        client_id: impl Into<String>,
        issuer: impl Into<String>,
        jwks_uri: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            issuer: issuer.into(),
            jwks_uri: jwks_uri.into(),
            jwks_cache: None,
        }
    }

    /// 从发现文档创建客户端
    pub async fn from_discovery(
        client_id: impl Into<String>,
        discovery_url: &str,
    ) -> Result<Self, OidcError> {
        let client = crate::http_client::get().clone();
        let doc: OidcDiscoveryDocument = client
            .get(discovery_url)
            .send()
            .await
            .map_err(|e| OidcError::HttpError(e.to_string()))?
            .json()
            .await
            .map_err(|e| OidcError::ParseError(e.to_string()))?;

        Ok(Self::new(client_id, doc.issuer, doc.jwks_uri))
    }

    /// 获取 JWKS (带缓存)
    pub async fn get_jwks(&mut self) -> Result<&JwksResponse, OidcError> {
        // 检查缓存是否有效 (缓存 1 小时)
        let should_fetch = match &self.jwks_cache {
            Some((_, cached_at)) => cached_at.elapsed().as_secs() >= 3600,
            None => true,
        };

        if should_fetch {
            // 获取新的 JWKS
            let client = crate::http_client::get().clone();
            let jwks: JwksResponse = client
                .get(&self.jwks_uri)
                .send()
                .await
                .map_err(|e| OidcError::HttpError(e.to_string()))?
                .json()
                .await
                .map_err(|e| OidcError::ParseError(e.to_string()))?;

            self.jwks_cache = Some((jwks, std::time::Instant::now()));
        }

        // 安全地返回缓存的引用
        Ok(&self.jwks_cache.as_ref().unwrap().0)
    }

    /// 验证 ID Token
    pub async fn verify_id_token(
        &mut self,
        id_token: &str,
        expected_nonce: Option<&str>,
    ) -> Result<IdTokenClaims, OidcError> {
        // 解码头部获取 kid
        let header = decode_header(id_token).map_err(|e| OidcError::InvalidToken(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| OidcError::InvalidToken("Missing kid in header".to_string()))?;

        // 获取对应的 JWK
        let jwks = self.get_jwks().await?;
        let jwk = jwks
            .keys
            .iter()
            .find(|k| k.kid.as_ref() == Some(&kid))
            .ok_or_else(|| OidcError::KeyNotFound(kid.clone()))?;

        // 构建解码密钥
        let decoding_key = jwk_to_decoding_key(jwk)?;

        // 配置验证
        let mut validation =
            Validation::new(algorithm_from_str(jwk.alg.as_deref().unwrap_or("RS256")));
        validation.set_audience(&[&self.client_id]);
        validation.set_issuer(&[&self.issuer]);

        // 解码并验证
        let token_data: TokenData<IdTokenClaims> = decode(id_token, &decoding_key, &validation)
            .map_err(|e| OidcError::InvalidToken(e.to_string()))?;

        let claims = token_data.claims;

        // 验证 nonce (如果提供)
        if let Some(expected) = expected_nonce {
            let actual = claims
                .nonce
                .as_deref()
                .ok_or_else(|| OidcError::InvalidToken("Missing nonce in token".to_string()))?;
            if actual != expected {
                return Err(OidcError::InvalidToken("Nonce mismatch".to_string()));
            }
        }

        Ok(claims)
    }
}

/// 将 JWK 转换为 DecodingKey
fn jwk_to_decoding_key(jwk: &Jwk) -> Result<DecodingKey, OidcError> {
    match jwk.kty.as_str() {
        "RSA" => {
            let n = jwk
                .n
                .as_ref()
                .ok_or_else(|| OidcError::InvalidKey("Missing n".to_string()))?;
            let e = jwk
                .e
                .as_ref()
                .ok_or_else(|| OidcError::InvalidKey("Missing e".to_string()))?;
            DecodingKey::from_rsa_components(n, e).map_err(|e| OidcError::InvalidKey(e.to_string()))
        }
        _ => Err(OidcError::InvalidKey(format!(
            "Unsupported key type: {}",
            jwk.kty
        ))),
    }
}

/// 将算法字符串转换为 Algorithm
fn algorithm_from_str(alg: &str) -> Algorithm {
    match alg {
        "RS256" => Algorithm::RS256,
        "RS384" => Algorithm::RS384,
        "RS512" => Algorithm::RS512,
        "ES256" => Algorithm::ES256,
        "ES384" => Algorithm::ES384,
        "HS256" => Algorithm::HS256,
        "HS384" => Algorithm::HS384,
        "HS512" => Algorithm::HS512,
        _ => Algorithm::RS256, // 默认使用 RS256
    }
}

/// 标准化用户信息
#[derive(Debug, Clone)]
pub struct NormalizedUserInfo {
    /// 外部用户 ID
    pub id: String,
    /// 邮箱
    pub email: Option<String>,
    /// 显示名称
    pub name: Option<String>,
    /// 头像 URL
    pub picture: Option<String>,
    /// 原始数据
    pub raw: serde_json::Value,
}

/// 从 ID Token Claims 提取用户信息
pub fn extract_user_info_from_id_token(claims: &IdTokenClaims) -> NormalizedUserInfo {
    NormalizedUserInfo {
        id: claims.sub.clone(),
        email: claims.email.clone(),
        name: claims
            .name
            .clone()
            .or_else(|| claims.preferred_username.clone()),
        picture: claims.picture.clone(),
        raw: serde_json::to_value(claims).unwrap_or_default(),
    }
}

/// 从 OAuth2 UserInfo 端点响应提取用户信息
pub fn extract_user_info_from_userinfo(
    data: &serde_json::Value,
    field_mapping: &serde_json::Value,
) -> NormalizedUserInfo {
    let get_field = |key: &str| -> Option<String> {
        let mapped_key = field_mapping
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(key);
        data.get(mapped_key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    NormalizedUserInfo {
        id: get_field("id").unwrap_or_default(),
        email: get_field("email"),
        name: get_field("name"),
        picture: get_field("picture"),
        raw: data.clone(),
    }
}

/// OIDC 错误类型
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid token: {0}")]
    InvalidToken(String),

    #[error("Key not found: {0}")]
    KeyNotFound(String),

    #[error("Invalid key: {0}")]
    InvalidKey(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_algorithm_from_str() {
        assert_eq!(algorithm_from_str("RS256"), Algorithm::RS256);
        assert_eq!(algorithm_from_str("RS384"), Algorithm::RS384);
        assert_eq!(algorithm_from_str("ES256"), Algorithm::ES256);
        assert_eq!(algorithm_from_str("HS256"), Algorithm::HS256);
        assert_eq!(algorithm_from_str("unknown"), Algorithm::RS256); // 默认值
    }

    #[test]
    fn test_extract_user_info_from_userinfo() {
        let data = serde_json::json!({
            "sub": "user123",
            "email": "user@example.com",
            "name": "Test User",
            "picture": "https://example.com/avatar.jpg"
        });

        let mapping = serde_json::json!({
            "id": "sub",
            "email": "email",
            "name": "name",
            "picture": "picture"
        });

        let info = extract_user_info_from_userinfo(&data, &mapping);
        assert_eq!(info.id, "user123");
        assert_eq!(info.email, Some("user@example.com".to_string()));
        assert_eq!(info.name, Some("Test User".to_string()));
        assert_eq!(
            info.picture,
            Some("https://example.com/avatar.jpg".to_string())
        );
    }

    #[test]
    fn test_extract_user_info_with_custom_mapping() {
        let data = serde_json::json!({
            "login": "testuser",
            "avatar_url": "https://github.com/avatar.png"
        });

        // GitHub 风格的字段映射
        let mapping = serde_json::json!({
            "id": "login",
            "email": "email",
            "name": "name",
            "picture": "avatar_url"
        });

        let info = extract_user_info_from_userinfo(&data, &mapping);
        assert_eq!(info.id, "testuser");
        assert_eq!(info.email, None);
        assert_eq!(
            info.picture,
            Some("https://github.com/avatar.png".to_string())
        );
    }
}
