//! SSO JWKS 客户端：从 SSO `.well-known/jwks.json` 动态获取 EC 公钥并缓存。
//!
//! 用于 Gateway PEP 验证 SSO 签发的 JWT，消除 Gateway 侧静态分发公钥（"去私钥"）。
//! 验证时按 token header 的 `kid` 选择对应 JWK；找不到或 JWKS 不可用时回退到
//! 静态配置的公钥（若存在）。

use std::sync::RwLock;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode_header, DecodingKey};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kty: String,
    // RFC 7517 JWK 标准字段：反序列化保留以校验/调试，验证时仅用 x/y/kid。
    #[serde(default)]
    #[allow(dead_code)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
    #[serde(default)]
    kid: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    alg: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

/// JWKS 解码错误分类（M69 审计 P3：kid 未命中是吊销信号，不得回退静态公钥）。
#[derive(Debug)]
pub enum JwksError {
    /// JWKS 获取/解析失败（网络、HTTP 状态、JSON 解析）——SSO 不可用，可回退静态公钥。
    FetchFailed(String),
    /// JWKS 已获取但无匹配密钥（kid 未命中 = 该 key 已从 JWKS 移除 = 吊销信号）。
    /// 回退静态公钥会让已吊销 key 签发的 token 继续通过 → fail-closed 拒绝。
    KeyNotFound,
    /// 其它验证性失败（非 EC、缺分量、构造失败）——同样 fail-closed。
    Unsupported(String),
}

impl std::fmt::Display for JwksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JwksError::FetchFailed(e) => write!(f, "JWKS 获取失败: {e}"),
            JwksError::KeyNotFound => write!(f, "JWKS 中无匹配密钥（kid 未命中）"),
            JwksError::Unsupported(e) => write!(f, "JWKS 密钥不支持: {e}"),
        }
    }
}

#[derive(Debug)]
pub struct SsoJwksClient {
    sso_base_url: String,
    http: reqwest::Client,
    cache: RwLock<Option<(JwksResponse, Instant)>>,
    ttl: Duration,
}

impl SsoJwksClient {
    pub fn new(sso_base_url: impl Into<String>) -> Self {
        Self {
            sso_base_url: sso_base_url.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
            cache: RwLock::new(None),
            ttl: Duration::from_secs(3600),
        }
    }

    async fn fetch_jwks(&self) -> Result<JwksResponse, String> {
        let url = format!("{}/.well-known/jwks.json", self.sso_base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("JWKS 请求失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("JWKS 端点返回 HTTP {}", resp.status()));
        }
        resp.json::<JwksResponse>()
            .await
            .map_err(|e| format!("JWKS 解析失败: {e}"))
    }

    async fn get_jwks(&self) -> Result<JwksResponse, String> {
        if let Ok(guard) = self.cache.read() {
            if let Some((jwks, fetched_at)) = guard.as_ref() {
                if fetched_at.elapsed() < self.ttl {
                    return Ok(jwks.clone());
                }
            }
        }
        let jwks = self.fetch_jwks().await?;
        if let Ok(mut guard) = self.cache.write() {
            *guard = Some((jwks.clone(), Instant::now()));
        }
        Ok(jwks)
    }

    /// 按 token header 的 `kid` 解析验证公钥；无 kid 时取第一个 EC 密钥。
    /// 错误分类见 `JwksError`：FetchFailed（可回退静态公钥）与 KeyNotFound/
    /// Unsupported（吊销/不支持信号，必须 fail-closed）严格区分。
    pub async fn decoding_key(&self, kid: &Option<String>) -> Result<DecodingKey, JwksError> {
        let jwks = self.get_jwks().await.map_err(JwksError::FetchFailed)?;
        let jwk = match kid {
            Some(kid) => jwks.keys.iter().find(|k| k.kid.as_deref() == Some(kid)),
            None => jwks.keys.first(),
        }
        .ok_or(JwksError::KeyNotFound)?;
        if jwk.kty != "EC" {
            return Err(JwksError::Unsupported(format!(
                "不支持的 JWK 类型: {}",
                jwk.kty
            )));
        }
        let x = jwk
            .x
            .as_ref()
            .ok_or_else(|| JwksError::Unsupported("JWK 缺少 x 分量".into()))?;
        let y = jwk
            .y
            .as_ref()
            .ok_or_else(|| JwksError::Unsupported("JWK 缺少 y 分量".into()))?;
        DecodingKey::from_ec_components(x, y)
            .map_err(|e| JwksError::Unsupported(format!("构造验证密钥失败: {e}")))
    }

    /// 解析 token header 的 `kid`（不验证签名）。
    pub fn token_kid(token: &str) -> Option<String> {
        decode_header(token).ok().and_then(|h| h.kid)
    }
}
