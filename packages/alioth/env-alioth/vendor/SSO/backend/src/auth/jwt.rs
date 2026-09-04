//! JWT authentication
//!
//! Provides JWT token generation, validation, and cookie management.
//! v10.0+ uses ES256 (EC P-256) asymmetric signatures: SSO holds the private key and
//! Gateway/SSO-internal verification uses the corresponding public key.

use actix_web::web;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// OIDC `id_token` Claims（OIDC Provider 对外签发的身份令牌）
///
/// 与内部 `Claims` 区分：id_token 面向外部依赖方（RP），必须携带 `iss`/`aud`/`nonce`，
/// 并复用同一 ES256 签名密钥与 JWKS，使 RP 可通过 `/.well-known/jwks.json` 验签。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcIdTokenClaims {
    /// Issuer — 必须等于 OIDC OP 的 issuer（config.oidc_issuer）
    pub iss: String,
    /// Subject — 用户标识（同内部 token 的 sub）
    pub sub: String,
    /// Audience — 接收方 client_id（防令牌错投）
    pub aud: String,
    /// Expiration timestamp
    pub exp: usize,
    /// Issued at timestamp
    pub iat: usize,
    /// 随机数（授权请求携带，防重放/伪装）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

/// JWT Claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject - user ID
    pub sub: String,
    /// Expiration timestamp
    pub exp: usize,
    /// Issued at timestamp
    pub iat: usize,
    /// User email
    #[serde(default)]
    pub email: String,
    /// MFA verification completed
    #[serde(default)]
    pub mfa_verified: bool,
    /// Protocol indicator (e.g. "zchat")
    #[serde(default)]
    pub protocol: String,
    /// JWT ID - unique identifier for this token (used to guarantee refresh token rotation uniqueness)
    #[serde(default)]
    pub jti: String,
    /// Session token linking this token to an SSO session.
    /// Enables Gateway PEP to validate session revocation status.
    /// Empty for tokens not bound to a persisted session.
    #[serde(default)]
    pub sid: String,
    /// Issuer — 必须等于 SSO 的 issuer（config.oidc_issuer）。
    /// 防止令牌被错投到其他部署/租户（即使共享同一公钥）。
    #[serde(default)]
    pub iss: String,
    /// Audience — 接收方标识（单第一方场景 == issuer）。
    /// 配合 `iss` 构成令牌绑定，验证方必须校验 `aud` 命中自身预期值。
    #[serde(default)]
    pub aud: String,
    /// 授予的 OAuth scope（空格分隔）；client_credentials 等服务令牌携带其子集。
    #[serde(default)]
    pub scope: String,
    /// 服务令牌主体：api_clients.fk_service_user（isahl_auth.auth_users 服务用户 id）。
    /// 自然人令牌为 0。Gateway PEP 以此作为 NGAC PDP 决策主体。
    #[serde(default)]
    #[serde(with = "common::serde_zuid")]
    pub svc_user_id: i64,
}

/// 令牌校验配置（iss/aud 绑定）。
///
/// 通过 `configure_token_validation` 在进程启动时注入（lib.rs 读取 Config 设置）；
/// 未配置时回退到默认（与 `Config::oidc_issuer` 默认值一致），保证测试与本地开发仍可验签。
/// 校验在 `decode_token` 中强制执行，缺失/不匹配即视为无效令牌。
#[derive(Debug, Clone)]
struct TokenValidationConfig {
    issuer: String,
    audience: String,
}

impl Default for TokenValidationConfig {
    fn default() -> Self {
        Self {
            issuer: "http://localhost:9002".to_string(),
            audience: "http://localhost:9002".to_string(),
        }
    }
}

static TOKEN_CONFIG: std::sync::OnceLock<TokenValidationConfig> = std::sync::OnceLock::new();

/// 注入预期的 iss/aud，供 encode 盖章与 decode 校验使用。
///
/// 必须在进程启动、签发/验证任何令牌之前调用一次（SSO `lib.rs`）。
/// 重复调用仅记录告警，不覆盖（首次注入为准）。
pub fn configure_token_validation(issuer: String, audience: String) {
    let cfg = TokenValidationConfig { issuer, audience };
    if TOKEN_CONFIG.set(cfg).is_err() {
        log::warn!("jwt: token validation already configured; ignoring duplicate configuration");
    }
}

/// 读取当前令牌校验配置（未配置则回退默认）。
fn token_config() -> TokenValidationConfig {
    TOKEN_CONFIG.get().cloned().unwrap_or_default()
}

/// 为 claims 盖章 iss/aud，确保签发出的令牌携带绑定声明。
fn stamp_audience(claims: &Claims) -> Claims {
    let cfg = token_config();
    let mut claims = claims.clone();
    claims.iss = cfg.issuer;
    claims.aud = cfg.audience;
    claims
}

impl Claims {
    /// 默认 Access Token TTL（秒），与 Config::jwt_access_expiry 默认值一致
    pub const DEFAULT_ACCESS_EXPIRY_SECS: i64 = 900;
    /// 默认 Refresh Token TTL（秒），与 Config::jwt_refresh_expiry 默认值一致
    pub const DEFAULT_REFRESH_EXPIRY_SECS: i64 = 604800;

    /// Create new claims for a user (default 15min access expiry)
    pub fn new(user_id: &str, email: &str, mfa_verified: bool) -> Self {
        Self::with_expiry_seconds(
            user_id,
            email,
            mfa_verified,
            Self::DEFAULT_ACCESS_EXPIRY_SECS,
        )
    }

    /// Create claims with custom expiry in seconds
    pub fn with_expiry_seconds(
        user_id: &str,
        email: &str,
        mfa_verified: bool,
        expiry_seconds: i64,
    ) -> Self {
        use chrono::TimeDelta;
        let now = Utc::now();

        Self {
            sub: user_id.to_string(),
            email: email.to_string(),
            exp: (now + TimeDelta::seconds(expiry_seconds)).timestamp() as usize,
            iat: now.timestamp() as usize,
            mfa_verified,
            protocol: String::new(),
            jti: String::new(),
            sid: String::new(),
            iss: String::new(),
            aud: String::new(),
            scope: String::new(),
            svc_user_id: 0,
        }
    }

    /// Create claims with custom expiry
    pub fn with_expiry(
        user_id: &str,
        email: &str,
        mfa_verified: bool,
        expiry_minutes: i64,
    ) -> Self {
        use chrono::TimeDelta;
        let now = Utc::now();

        Self {
            sub: user_id.to_string(),
            email: email.to_string(),
            exp: (now + TimeDelta::minutes(expiry_minutes)).timestamp() as usize,
            iat: now.timestamp() as usize,
            mfa_verified,
            protocol: String::new(),
            jti: String::new(),
            sid: String::new(),
            iss: String::new(),
            aud: String::new(),
            scope: String::new(),
            svc_user_id: 0,
        }
    }

    /// Create temporary JWT for identity-only access (24h expiry).
    pub fn temp(user_id: &str, email: &str) -> Self {
        Self::with_expiry(user_id, email, false, 24 * 60)
    }
}

/// JWT errors
#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("Token decode error: {0}")]
    DecodeError(#[from] jsonwebtoken::errors::Error),
    #[error("Key error: {0}")]
    KeyError(String),
}

/// Actix auth errors
#[derive(Debug, thiserror::Error)]
pub enum ActixAuthError {
    #[error("Missing authorization token")]
    MissingToken,
    #[error("Invalid token format")]
    InvalidTokenFormat,
    #[error("Invalid or expired token")]
    InvalidToken,
}

impl From<JwtError> for ActixAuthError {
    fn from(_: JwtError) -> Self {
        ActixAuthError::InvalidToken
    }
}

/// Derive an ES256 (EC P-256) public key (PEM) from a PKCS#8 private key (PEM).
pub fn derive_public_key(private_key_pem: &[u8]) -> Result<Vec<u8>, JwtError> {
    use p256::pkcs8::{DecodePrivateKey, EncodePublicKey};

    let pem_str =
        std::str::from_utf8(private_key_pem).map_err(|e| JwtError::KeyError(e.to_string()))?;
    let private_key =
        p256::SecretKey::from_pkcs8_pem(pem_str).map_err(|e| JwtError::KeyError(e.to_string()))?;
    let public_key = private_key.public_key();
    let public_pem = public_key
        .to_public_key_pem(p256::pkcs8::LineEnding::LF)
        .map_err(|e| JwtError::KeyError(e.to_string()))?;
    Ok(public_pem.into_bytes())
}

/// 由 EC 公钥 (PEM) 计算稳定 `kid`，用于 JWT header 与 JWKS 密钥的标识匹配。
///
/// 算法：公钥 DER 的 SHA-256 → base64url（无填充）截断 16 字符。
/// 签发端（encode_*）与服务端（JWKS 端点）必须使用同一算法，否则 kid 无法对应。
pub fn public_key_kid(public_key_pem: &[u8]) -> Result<String, JwtError> {
    use base64::Engine;
    use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
    use sha2::{Digest, Sha256};

    let pem_str =
        std::str::from_utf8(public_key_pem).map_err(|e| JwtError::KeyError(e.to_string()))?;
    let pk = p256::PublicKey::from_public_key_pem(pem_str)
        .map_err(|e| JwtError::KeyError(e.to_string()))?;
    let der = pk
        .to_public_key_der()
        .map_err(|e| JwtError::KeyError(e.to_string()))?;
    let hash = Sha256::digest(der.as_bytes());
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)[..16].to_string())
}

/// 由 EC 公钥 (PEM) 构造 JWK（RFC 7517），用于 JWKS 端点分发。
///
/// 仅支持 EC P-256（与 ES256 签发算法一致）。
pub fn public_key_to_jwk(public_key_pem: &[u8], kid: &str) -> Result<serde_json::Value, JwtError> {
    use base64::Engine;
    use p256::elliptic_curve::sec1::ToSec1Point;
    use p256::pkcs8::DecodePublicKey;

    let pem_str =
        std::str::from_utf8(public_key_pem).map_err(|e| JwtError::KeyError(e.to_string()))?;
    let pk = p256::PublicKey::from_public_key_pem(pem_str)
        .map_err(|e| JwtError::KeyError(e.to_string()))?;
    let point = pk.to_sec1_point(false);
    let bytes = point.as_bytes();
    // 非压缩点: 0x04 || X(32) || Y(32)
    if bytes.len() != 65 || bytes[0] != 0x04 {
        return Err(JwtError::KeyError("unexpected EC point length".into()));
    }
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let x = enc.encode(&bytes[1..33]);
    let y = enc.encode(&bytes[33..65]);
    Ok(serde_json::json!({
        "kty": "EC",
        "crv": "P-256",
        "x": x,
        "y": y,
        "use": "sig",
        "alg": "ES256",
        "kid": kid,
    }))
}

/// JWKS 端点：分发 SSO 签名公钥（EC P-256），供 Gateway / 依赖方动态获取并验证 JWT。
///
/// 返回 RFC 7517 JWK Set（active + 轮换窗口 prev 多 key，各自 kid）。
/// 配合 JWT header 中的 `kid`，验证方无需静态分发公钥。
pub async fn jwks(state: web::Data<crate::auth::AuthState>) -> HttpResponse {
    let mut keys = Vec::with_capacity(1 + state.jwt_public_keys_prev.len());

    let active_kid = match public_key_kid(&state.jwt_public_key) {
        Ok(k) => k,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": e.to_string() }))
        }
    };
    match public_key_to_jwk(&state.jwt_public_key, &active_kid) {
        Ok(j) => keys.push(j),
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": e.to_string() }))
        }
    }

    // 轮换窗口：导出 prev 公钥（重算 kid，不信任存储值）；单把失败不整体失败
    for (_, prev_pem) in &state.jwt_public_keys_prev {
        let prev_kid = match public_key_kid(prev_pem) {
            Ok(k) => k,
            Err(e) => {
                log::warn!("jwks: prev public key kid derivation failed: {e}");
                continue;
            }
        };
        match public_key_to_jwk(prev_pem, &prev_kid) {
            Ok(j) => keys.push(j),
            Err(e) => {
                log::warn!("jwks: prev public key to_jwk failed: {e}");
                continue;
            }
        }
    }

    HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::json!({ "keys": keys }))
}

/// Validate access token from request (Cookie first, then Authorization header)
pub async fn validate_access_token(
    req: &HttpRequest,
    public_keys: &[&[u8]],
) -> Result<Claims, ActixAuthError> {
    let token = extract_token(req).ok_or(ActixAuthError::MissingToken)?;
    let claims = decode_token_any(&token, public_keys).map_err(|_| ActixAuthError::InvalidToken)?;
    Ok(claims)
}

/// Decode and validate a JWT token using the ES256 public key.
///
/// 除 `exp` 外，还会强制校验 `iss` 与 `aud`（来自 `configure_token_validation`），
/// 防止令牌跨部署/跨服务错投被接受。
pub fn decode_token(token: &str, public_key: &[u8]) -> Result<Claims, JwtError> {
    let cfg = token_config();
    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    validation.set_issuer(&[&cfg.issuer]);
    validation.set_audience(&[&cfg.audience]);

    let token_data = decode::<Claims>(token, &DecodingKey::from_ec_pem(public_key)?, &validation)?;

    Ok(token_data.claims)
}

/// Decode and validate a JWT token, trying each public key in order.
///
/// 轮换窗口内 active + prev 依次尝试（SSO 内部验签路径多 key 化）；
/// 全部失败返回第一个错误（保持单钥错误语义），空列表返回 KeyError。
pub fn decode_token_any(token: &str, public_keys: &[&[u8]]) -> Result<Claims, JwtError> {
    let mut first_err: Option<JwtError> = None;
    for key in public_keys {
        match decode_token(token, key) {
            Ok(c) => return Ok(c),
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    Err(first_err.unwrap_or_else(|| {
        JwtError::KeyError("no public key configured for verification".to_string())
    }))
}

/// Get access token expiry timestamp
pub fn get_access_token_expiry() -> usize {
    use chrono::TimeDelta;
    (Utc::now() + TimeDelta::minutes(15)).timestamp() as usize
}

/// Get access token expiry timestamp with custom duration
pub fn get_access_token_expiry_minutes(minutes: i64) -> usize {
    use chrono::TimeDelta;
    (Utc::now() + TimeDelta::minutes(minutes)).timestamp() as usize
}

/// Get current timestamp
pub fn get_current_timestamp() -> usize {
    Utc::now().timestamp() as usize
}

/// Encode access token with the ES256 private key.
pub fn encode_access_token(claims: &Claims, private_key: &[u8]) -> Result<String, JwtError> {
    let claims = stamp_audience(claims);
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(public_key_kid(&derive_public_key(private_key)?)?);
    encode(&header, &claims, &EncodingKey::from_ec_pem(private_key)?).map_err(JwtError::DecodeError)
}

/// Encode refresh token with the ES256 private key.
///
/// 为每次刷新生成唯一的 `jti`，避免在同一秒内生成的 refresh token 与旧 token
/// 完全相同，从而导致 rotation 后新 token 实际已被撤销的问题。
pub fn encode_refresh_token(
    claims: &Claims,
    private_key: &[u8],
    refresh_expiry_secs: i64,
) -> Result<String, JwtError> {
    use chrono::TimeDelta;
    let refresh_claims = Claims {
        sub: claims.sub.clone(),
        email: claims.email.clone(),
        exp: (Utc::now() + TimeDelta::seconds(refresh_expiry_secs)).timestamp() as usize,
        iat: get_current_timestamp(),
        mfa_verified: false,
        protocol: String::new(),
        jti: uuid::Uuid::new_v4().to_string(),
        sid: claims.sid.clone(),
        iss: String::new(),
        aud: String::new(),
        scope: String::new(),
        svc_user_id: claims.svc_user_id,
    };
    let refresh_claims = stamp_audience(&refresh_claims);
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(public_key_kid(&derive_public_key(private_key)?)?);
    encode(
        &header,
        &refresh_claims,
        &EncodingKey::from_ec_pem(private_key)?,
    )
    .map_err(JwtError::DecodeError)
}

/// Encode temporary access token (24h expiry for identity flow) with the ES256 private key.
pub fn encode_temp_token(claims: &Claims, private_key: &[u8]) -> Result<String, JwtError> {
    let claims = stamp_audience(claims);
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(public_key_kid(&derive_public_key(private_key)?)?);
    encode(&header, &claims, &EncodingKey::from_ec_pem(private_key)?).map_err(JwtError::DecodeError)
}

/// 以 ES256 签发 OIDC `id_token`（复用 SSO 签名私钥与 kid 算法，RP 经 JWKS 验签）。
pub fn encode_id_token(claims: &OidcIdTokenClaims, private_key: &[u8]) -> Result<String, JwtError> {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(public_key_kid(&derive_public_key(private_key)?)?);
    encode(&header, claims, &EncodingKey::from_ec_pem(private_key)?).map_err(JwtError::DecodeError)
}

/// Refresh access token using refresh token claims
///
/// Validates the refresh token and returns new access token with updated claims
pub fn refresh_access_token(
    refresh_claims: &Claims,
    private_key: &[u8],
    access_expiry_secs: i64,
) -> Result<(String, Claims), JwtError> {
    use chrono::TimeDelta;

    // Validate refresh token hasn't expired
    let now = Utc::now().timestamp() as usize;
    if refresh_claims.exp < now {
        return Err(JwtError::DecodeError(jsonwebtoken::errors::Error::from(
            jsonwebtoken::errors::ErrorKind::ExpiredSignature,
        )));
    }

    // Create new access token claims based on refresh token
    let access_claims = Claims {
        sub: refresh_claims.sub.clone(),
        email: refresh_claims.email.clone(),
        exp: (Utc::now() + TimeDelta::seconds(access_expiry_secs)).timestamp() as usize,
        iat: get_current_timestamp(),
        mfa_verified: refresh_claims.mfa_verified,
        protocol: refresh_claims.protocol.clone(),
        jti: String::new(),
        sid: refresh_claims.sid.clone(),
        scope: refresh_claims.scope.clone(),
        iss: String::new(),
        aud: String::new(),
        svc_user_id: refresh_claims.svc_user_id,
    };
    let access_claims = stamp_audience(&access_claims);

    let access_token = encode_access_token(&access_claims, private_key)?;

    Ok((access_token, access_claims))
}

/// Set refresh token cookie
use cookie::Cookie;
use time::Duration;

pub fn set_access_cookie(response: HttpResponse, token: &str, max_age_secs: i64) -> HttpResponse {
    set_cookie(
        response,
        "access_token",
        token,
        "/",
        Duration::seconds(max_age_secs),
    )
}

pub fn clear_access_cookie(response: HttpResponse) -> HttpResponse {
    clear_cookie(response, "access_token", "/")
}

pub fn set_refresh_cookie(response: HttpResponse, token: &str, max_age_secs: i64) -> HttpResponse {
    set_cookie(
        response,
        "refresh_token",
        token,
        "/api/auth/refresh",
        Duration::seconds(max_age_secs),
    )
}

pub fn clear_refresh_cookie(response: HttpResponse) -> HttpResponse {
    clear_cookie(response, "refresh_token", "/api/auth/refresh")
}

/// Whether auth cookies should carry the Secure flag.
/// Defaults to false so that HTTP deployments (including the bundled frontend
/// server) work out of the box. Set COOKIE_SECURE=true explicitly when running
/// behind an HTTPS reverse proxy.
fn cookie_secure() -> bool {
    std::env::var("COOKIE_SECURE")
        .ok()
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Choose SameSite policy based on cookie Secure flag.
///
/// Development (non-HTTPS, Secure=false) uses Lax so that cross-port requests on
/// localhost still carry the cookie, preventing repeated login prompts after page
/// refresh. Production (HTTPS, Secure=true) uses Strict for stronger CSRF protection.
fn cookie_same_site() -> cookie::SameSite {
    if cookie_secure() {
        cookie::SameSite::Strict
    } else {
        cookie::SameSite::Lax
    }
}

fn set_cookie(
    mut response: HttpResponse,
    name: &str,
    value: &str,
    path: &str,
    max_age: Duration,
) -> HttpResponse {
    use actix_web::http::header::SET_COOKIE;
    let mut cookie = Cookie::new(name, value);
    cookie.set_path(path);
    cookie.set_http_only(true);
    cookie.set_secure(cookie_secure());
    cookie.set_same_site(cookie_same_site());
    cookie.set_max_age(max_age);
    // Use append() so multiple Set-Cookie headers coexist (access_token + refresh_token)
    response
        .headers_mut()
        .append(SET_COOKIE, cookie.to_string().parse().unwrap());
    response
}

fn clear_cookie(mut response: HttpResponse, name: &str, path: &str) -> HttpResponse {
    use actix_web::http::header::SET_COOKIE;
    let mut cookie = Cookie::new(name, "");
    cookie.set_path(path);
    cookie.set_http_only(true);
    cookie.set_secure(cookie_secure());
    cookie.set_same_site(cookie_same_site());
    cookie.set_max_age(Duration::ZERO);
    response
        .headers_mut()
        .append(SET_COOKIE, cookie.to_string().parse().unwrap());
    response
}

/// Extract JWT access token from Cookie, falling back to Authorization header.
pub fn extract_token(req: &HttpRequest) -> Option<String> {
    // 1. Try httpOnly cookie first (preferred)
    if let Some(cookie) = req.cookie("access_token") {
        let value = cookie.value().trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }

    // 2. Fall back to Authorization header for backward compatibility
    req.headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 测试用 EC P-256 密钥对（PKCS#8 PEM）
    const TEST_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs
\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0
\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm
\
-----END PRIVATE KEY-----";

    const TEST_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAExHBYWD4VZBXSBjQIgcMUKbtHGEV3
\
NK6CQd0RxdS3yLGgsXJ1XqdLJwuPvErVIsSI3ywGfDHPPrqmuN53XjRBZg==
\
-----END PUBLIC KEY-----";

    #[test]
    fn test_derive_public_key_matches_test_public_key() {
        let derived = derive_public_key(TEST_PRIVATE_KEY).unwrap();
        assert_eq!(
            std::str::from_utf8(&derived).unwrap().trim(),
            std::str::from_utf8(TEST_PUBLIC_KEY).unwrap().trim()
        );
    }

    #[test]
    fn test_public_key_kid_stable_and_matches_token_header() {
        let kid = public_key_kid(TEST_PUBLIC_KEY).unwrap();
        assert!(!kid.is_empty());
        // 同一公钥派生出的 kid 必须幂等（签发端与 JWKS 端一致）
        assert_eq!(kid, public_key_kid(TEST_PUBLIC_KEY).unwrap());

        let claims = Claims::new("u", "e@x.com", false);
        let token = encode_access_token(&claims, TEST_PRIVATE_KEY).unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.kid.as_deref(), Some(kid.as_str()));
        assert_eq!(header.alg, jsonwebtoken::Algorithm::ES256);
    }

    #[test]
    fn test_public_key_to_jwk_validates_signed_token() {
        let kid = public_key_kid(TEST_PUBLIC_KEY).unwrap();
        let jwk = public_key_to_jwk(TEST_PUBLIC_KEY, &kid).unwrap();
        assert_eq!(jwk["kty"], "EC");
        assert_eq!(jwk["crv"], "P-256");
        assert_eq!(jwk["alg"], "ES256");
        assert_eq!(jwk["kid"], kid);

        let x = jwk["x"].as_str().unwrap();
        let y = jwk["y"].as_str().unwrap();
        // 用 JWK 的 x/y 重建 DecodingKey，须能验证对应私钥签发的 token
        let key = jsonwebtoken::DecodingKey::from_ec_components(x, y).unwrap();
        let claims = Claims::new("u", "e@x.com", false);
        let token = encode_access_token(&claims, TEST_PRIVATE_KEY).unwrap();
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.validate_exp = true;
        // 令牌现强制携带 iss/aud（默认配置 http://localhost:9002），校验须匹配
        validation.set_issuer(&["http://localhost:9002"]);
        validation.set_audience(&["http://localhost:9002"]);
        let decoded = jsonwebtoken::decode::<Claims>(&token, &key, &validation).unwrap();
        assert_eq!(decoded.claims.sub, "u");
    }

    #[test]
    fn test_access_token_encode_decode() {
        let claims = Claims::new("user123", "user@example.com", true);

        let token = encode_access_token(&claims, TEST_PRIVATE_KEY).unwrap();

        let decoded = decode_token(&token, TEST_PUBLIC_KEY).unwrap();

        assert_eq!(decoded.sub, "user123");
        assert_eq!(decoded.email, "user@example.com");
        assert!(decoded.mfa_verified);
    }

    #[test]
    fn test_expired_token() {
        let mut claims = Claims::new("user123", "user@example.com", true);
        claims.exp = (Utc::now() - chrono::Duration::hours(1)).timestamp() as usize;

        let token = encode_access_token(&claims, TEST_PRIVATE_KEY).unwrap();

        let result = decode_token(&token, TEST_PUBLIC_KEY);
        assert!(result.is_err());
    }

    #[test]
    fn test_with_expiry_seconds_respects_configured_ttl() {
        let before = Utc::now().timestamp() as usize;
        let claims = Claims::with_expiry_seconds("user123", "user@example.com", false, 60);
        // exp 应落在 [before+60, now+60] 区间
        assert!(claims.exp >= before + 60);
        assert!(claims.exp <= (Utc::now().timestamp() as usize) + 60);
    }

    #[test]
    fn test_claims_new_matches_default_access_ttl() {
        let before = Utc::now().timestamp() as usize;
        let claims = Claims::new("user123", "user@example.com", true);
        assert!(claims.exp >= before + Claims::DEFAULT_ACCESS_EXPIRY_SECS as usize);
    }

    #[test]
    fn test_refresh_token_uses_configured_ttl() {
        let claims = Claims::new("user123", "user@example.com", true);
        let before = Utc::now().timestamp() as usize;
        let token = encode_refresh_token(&claims, TEST_PRIVATE_KEY, 3600).unwrap();
        let decoded = decode_token(&token, TEST_PUBLIC_KEY).unwrap();
        assert!(decoded.exp >= before + 3600);
        assert!(!decoded.jti.is_empty());
    }

    // 第二把 EC P-256 密钥对（模拟轮换后的 prev 钥，与 TEST_* 不同源）
    const TEST_PREV_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgkUXkbsFeGTxRAuvY
\
FRUoGkTqPKKLYJ+1c5A4sQbKDlChRANCAAQujoQbf1/KuEBy50ws3vLxtBszp+wj
\
t3ac6CVz6zQl8Vb7sTqje0wGgbP8auaIsYof1dX4B6PM2FglnfMScRaP
\
-----END PRIVATE KEY-----";

    const TEST_PREV_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAELo6EG39fyrhAcudMLN7y8bQbM6fs
\
I7d2nOglc+s0JfFW+7E6o3tMBoGz/GrmiLGKH9XV+AejzNhYJZ3zEnEWjw==
\
-----END PUBLIC KEY-----";

    #[test]
    fn test_decode_token_any_active_first() {
        let claims = Claims::new("u", "e@x.com", false);
        let token = encode_access_token(&claims, TEST_PRIVATE_KEY).unwrap();
        // active 正确 + prev 正确：active 优先成功
        let decoded = decode_token_any(&token, &[TEST_PUBLIC_KEY, TEST_PREV_PUBLIC_KEY]).unwrap();
        assert_eq!(decoded.sub, "u");
    }

    #[test]
    fn test_decode_token_any_prev_fallback() {
        // 轮换场景：token 由旧钥（prev）签发，active 无法验，prev 兜底
        let claims = Claims::new("u", "e@x.com", false);
        let token = encode_access_token(&claims, TEST_PREV_PRIVATE_KEY).unwrap();
        let decoded = decode_token_any(&token, &[TEST_PUBLIC_KEY, TEST_PREV_PUBLIC_KEY]).unwrap();
        assert_eq!(decoded.sub, "u");
    }

    #[test]
    fn test_decode_token_any_all_fail() {
        let claims = Claims::new("u", "e@x.com", false);
        let token = encode_access_token(&claims, TEST_PRIVATE_KEY).unwrap();
        // 仅错误钥 → 失败；空列表 → 失败
        assert!(decode_token_any(&token, &[TEST_PREV_PUBLIC_KEY]).is_err());
        assert!(decode_token_any(&token, &[]).is_err());
    }

    #[tokio::test]
    async fn test_jwks_exports_active_and_prev() {
        let state = crate::auth::AuthState {
            jwt_private_key: TEST_PRIVATE_KEY.to_vec(),
            jwt_public_key: derive_public_key(TEST_PRIVATE_KEY).unwrap(),
            jwt_public_keys_prev: vec![(
                public_key_kid(TEST_PREV_PUBLIC_KEY).unwrap(),
                TEST_PREV_PUBLIC_KEY.to_vec(),
            )],
            encryption_key: vec![],
            ngac_preview_dir: None,
            jwt_access_expiry_secs: 900,
            jwt_refresh_expiry_secs: 604800,
            identity_verify_mode: "local".to_string(),
            identity_external_verify_url: None,
        };
        let data = actix_web::web::Data::new(state);
        let resp = crate::auth::jwt::jwks(data).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let keys = json["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2, "JWKS 应导出 active + prev 两把钥");
        let kids: Vec<&str> = keys.iter().map(|k| k["kid"].as_str().unwrap()).collect();
        assert!(kids.contains(&public_key_kid(TEST_PUBLIC_KEY).unwrap().as_str()));
        assert!(kids.contains(&public_key_kid(TEST_PREV_PUBLIC_KEY).unwrap().as_str()));
    }

    #[test]
    fn test_verification_keys_active_then_prev() {
        let state = crate::auth::AuthState {
            jwt_private_key: TEST_PRIVATE_KEY.to_vec(),
            jwt_public_key: derive_public_key(TEST_PRIVATE_KEY).unwrap(),
            jwt_public_keys_prev: vec![(
                public_key_kid(TEST_PREV_PUBLIC_KEY).unwrap(),
                TEST_PREV_PUBLIC_KEY.to_vec(),
            )],
            encryption_key: vec![],
            ngac_preview_dir: None,
            jwt_access_expiry_secs: 900,
            jwt_refresh_expiry_secs: 604800,
            identity_verify_mode: "local".to_string(),
            identity_external_verify_url: None,
        };
        let keys = state.verification_keys();
        assert_eq!(keys.len(), 2);
        // active 在前，prev 在后
        assert_eq!(
            String::from_utf8_lossy(keys[0]).trim(),
            String::from_utf8_lossy(&derive_public_key(TEST_PRIVATE_KEY).unwrap()).trim()
        );
        assert_eq!(keys[1], TEST_PREV_PUBLIC_KEY);
    }
}
