//! SSO JWKS 客户端：从 SSO `.well-known/jwks.json` 动态获取 EC 公钥并缓存。
//!
//! 用于 Gateway PEP 验证 SSO 签发的 JWT，消除 Gateway 侧静态分发公钥（"去私钥"）。
//! 验证时按 token header 的 `kid` 选择对应 JWK；找不到或 JWKS 不可用时回退到
//! 静态配置的公钥（若存在）。
//!
//! 缓存形态：拉取时一次性把 JWK 物化为 `DecodingKey`（`Arc`），请求内仅按 kid 取
//! 引用——避免每请求重跑 base64 分量解码 + EC 点构造 + 整包 JSON 深拷贝。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use jsonwebtoken::{decode_header, DecodingKey};
use serde::Deserialize;

/// JWKS 拉取超时——与 `HttpNgacClient`（`ngac-contract`）同口径：PDP/JWKS 目标恒为
/// 本地 SSO 回环，缺失超时会让 SSO 无响应时请求线程悬挂在 TCP 层。
const JWKS_FETCH_TIMEOUT: Duration = Duration::from_secs(3);

/// 拉取失败负缓存窗口——SSO 不可用期间的请求直接走静态公钥回退，
/// 不再每请求重试一次网络（恢复延迟上界 = 本窗口）。
const JWKS_NEGATIVE_TTL: Duration = Duration::from_secs(5);

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
            Self::FetchFailed(e) => write!(f, "JWKS 获取失败: {e}"),
            Self::KeyNotFound => write!(f, "JWKS 中找不到匹配 kid 的密钥"),
            Self::Unsupported(e) => write!(f, "JWKS 密钥不可用: {e}"),
        }
    }
}

/// 单个 JWK 的物化结果：成功构造的验证密钥，或不可用原因（保留 fail-closed 语义）。
#[derive(Debug, Clone)]
enum CachedKey {
    Key(Arc<DecodingKey>),
    Unsupported(String),
}

/// 一次拉取后的密钥表（`Arc` 共享，请求间零深拷贝）。
#[derive(Debug)]
struct JwksCache {
    /// kid → 密钥（首次出现者优先，与既有 `keys.iter().find(..)` 语义一致）。
    by_kid: HashMap<String, CachedKey>,
    /// 无 kid 令牌的缺省密钥 = JWKS 首个条目的物化结果（与既有 `keys.first()` 一致）。
    default: Option<CachedKey>,
    fetched_at: Instant,
}

#[derive(Debug, Default)]
struct JwksState {
    cache: Option<Arc<JwksCache>>,
    /// 上次拉取失败时刻（负缓存窗口内不重试）。
    failed_at: Option<Instant>,
}

#[derive(Debug)]
pub struct SsoJwksClient {
    sso_base_url: String,
    state: RwLock<JwksState>,
    ttl: Duration,
}

impl SsoJwksClient {
    pub fn new(sso_base_url: impl Into<String>) -> Self {
        Self {
            sso_base_url: sso_base_url.into().trim_end_matches('/').to_string(),
            state: RwLock::new(JwksState::default()),
            ttl: Duration::from_secs(3600),
        }
    }

    async fn fetch_jwks(&self) -> Result<JwksResponse, String> {
        let url = format!("{}/.well-known/jwks.json", self.sso_base_url);
        // NGAC/JWKS 目标恒为本地 SSO 回环——禁 env 代理劫持（与 HttpNgacClient 同口径）
        let client = reqwest::Client::builder()
            .timeout(JWKS_FETCH_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|e| format!("JWKS HTTP 客户端构建失败: {e}"))?;
        let resp = client
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

    /// 物化单条 JWK（与原内联逻辑一致：非 EC / 缺分量 / 构造失败 → Unsupported）。
    fn build_key(jwk: &Jwk) -> Result<DecodingKey, String> {
        if jwk.kty != "EC" {
            return Err(format!("不支持的 JWK 类型: {}", jwk.kty));
        }
        let x = jwk
            .x
            .as_ref()
            .ok_or_else(|| "JWK 缺少 x 分量".to_string())?;
        let y = jwk
            .y
            .as_ref()
            .ok_or_else(|| "JWK 缺少 y 分量".to_string())?;
        DecodingKey::from_ec_components(x, y).map_err(|e| format!("构造验证密钥失败: {e}"))
    }

    /// 由 JWKS 响应构建密钥表（纯函数，便于单测）。
    fn build_cache(keys: &[Jwk]) -> JwksCache {
        let mut by_kid: HashMap<String, CachedKey> = HashMap::new();
        for jwk in keys {
            let Some(kid) = jwk.kid.as_ref() else {
                continue;
            };
            let entry = match Self::build_key(jwk) {
                Ok(key) => CachedKey::Key(Arc::new(key)),
                Err(msg) => CachedKey::Unsupported(msg),
            };
            // 同 kid 重复出现时保留首个（对齐既有 find 语义）
            by_kid.entry(kid.clone()).or_insert(entry);
        }
        let default = keys.first().map(|jwk| match Self::build_key(jwk) {
            Ok(key) => CachedKey::Key(Arc::new(key)),
            Err(msg) => CachedKey::Unsupported(msg),
        });
        JwksCache {
            by_kid,
            default,
            fetched_at: Instant::now(),
        }
    }

    /// 取当前密钥表：TTL 内直接复用；过期则重拉；失败进入负缓存窗口。
    async fn get_cache(&self) -> Result<Arc<JwksCache>, String> {
        {
            let state = self.state.read().unwrap_or_else(|p| p.into_inner());
            if let Some(cache) = state.cache.as_ref() {
                if cache.fetched_at.elapsed() < self.ttl {
                    return Ok(Arc::clone(cache));
                }
            }
            if let Some(failed_at) = state.failed_at {
                if failed_at.elapsed() < JWKS_NEGATIVE_TTL {
                    return Err("JWKS 拉取失败（负缓存窗口内不重试）".to_string());
                }
            }
        }

        match self.fetch_jwks().await {
            Ok(jwks) => {
                let cache = Arc::new(Self::build_cache(&jwks.keys));
                let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
                state.cache = Some(Arc::clone(&cache));
                state.failed_at = None;
                Ok(cache)
            }
            Err(e) => {
                let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
                state.failed_at = Some(Instant::now());
                Err(e)
            }
        }
    }

    /// 按 token header 的 `kid` 解析验证公钥；无 kid 时取 JWKS 首条。
    /// 错误分类见 `JwksError`：FetchFailed（可回退静态公钥）与 KeyNotFound/
    /// Unsupported（吊销/不支持信号，必须 fail-closed）严格区分。
    pub async fn decoding_key(&self, kid: &Option<String>) -> Result<Arc<DecodingKey>, JwksError> {
        let cache = self.get_cache().await.map_err(JwksError::FetchFailed)?;
        let entry = match kid {
            Some(kid) => cache.by_kid.get(kid),
            None => cache.default.as_ref(),
        };
        match entry {
            Some(CachedKey::Key(key)) => Ok(Arc::clone(key)),
            Some(CachedKey::Unsupported(msg)) => Err(JwksError::Unsupported(msg.clone())),
            None => Err(JwksError::KeyNotFound),
        }
    }

    /// 解析 token header 的 `kid`（不验证签名）。
    pub fn token_kid(token: &str) -> Option<String> {
        decode_header(token).ok().and_then(|h| h.kid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7515 A.3.1 的 P-256 示例坐标（公开测试向量）。
    const X: &str = "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU";
    const Y: &str = "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0";

    fn jwk(kid: Option<&str>, kty: &str, x: Option<&str>, y: Option<&str>) -> Jwk {
        Jwk {
            kty: kty.to_string(),
            crv: Some("P-256".to_string()),
            x: x.map(str::to_string),
            y: y.map(str::to_string),
            kid: kid.map(str::to_string),
            alg: Some("ES256".to_string()),
        }
    }

    #[test]
    fn decoding_key_resolves_by_kid_without_rebuild() {
        let cache = SsoJwksClient::build_cache(&[
            jwk(Some("other"), "EC", Some(X), Some(Y)),
            jwk(Some("k1"), "EC", Some(X), Some(Y)),
        ]);
        let first = match cache.by_kid.get("k1") {
            Some(CachedKey::Key(k)) => Arc::clone(k),
            other => panic!("expected cached EC key, got {other:?}"),
        };
        // 同一 kid 二次查找命中同一 Arc（无重建、无克隆）
        let second = match cache.by_kid.get("k1") {
            Some(CachedKey::Key(k)) => Arc::clone(k),
            other => panic!("expected cached EC key, got {other:?}"),
        };
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn decoding_key_missing_kid_entry_is_key_not_found() {
        let cache = SsoJwksClient::build_cache(&[jwk(Some("k1"), "EC", Some(X), Some(Y))]);
        assert!(cache.by_kid.get("absent").is_none());
        // 首条物化为缺省密钥（无 kid 令牌路径）
        assert!(matches!(cache.default, Some(CachedKey::Key(_))));
    }

    #[test]
    fn non_ec_kid_stays_fail_closed() {
        let cache = SsoJwksClient::build_cache(&[jwk(Some("rsa1"), "RSA", Some(X), Some(Y))]);
        match cache.by_kid.get("rsa1") {
            Some(CachedKey::Unsupported(msg)) => assert!(msg.contains("RSA")),
            other => panic!("expected Unsupported entry, got {other:?}"),
        }
        // 无 kid 令牌同样 fail-closed（缺省 = 首条，非 EC）
        assert!(matches!(cache.default, Some(CachedKey::Unsupported(_))));
    }

    #[test]
    fn duplicate_kid_keeps_first_entry() {
        let cache = SsoJwksClient::build_cache(&[
            jwk(Some("dup"), "RSA", Some(X), Some(Y)),
            jwk(Some("dup"), "EC", Some(X), Some(Y)),
        ]);
        assert!(matches!(
            cache.by_kid.get("dup"),
            Some(CachedKey::Unsupported(_))
        ));
    }
}
