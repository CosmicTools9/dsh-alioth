//! OIDC Provider（OP）端点实现
//!
//! 对外提供最小可用的 OIDC 1.0 `authorization_code` 流程：
//! - `GET /.well-known/openid-configuration`：发现文档（声明 issuer / 端点 / ES256）
//! - `GET /oidc/authorize`：已认证用户 → 签发短期单次 `authorization_code`，重定向回白名单 `redirect_uri`
//! - `POST /oidc/token`：用 `code` 兑换 ES256 签名的 `id_token`（RP 经既有 JWKS 验签）
//!
//! 设计约束（SECURITY_SPEC 新增「OIDC OP」章节）：
//! - `id_token` 复用 SSO 既有 ES256 私钥与 JWKS，禁止引入第二套签名密钥
//! - `redirect_uri` 强制校验白名单，防开放重定向
//! - `authorization_code` 单次使用且短时效（默认 60s），防重放
//! - `nonce` 透传至 `id_token`，供 RP 绑定授权请求

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use actix_web::http::header;
use actix_web::web;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use common::ErrorResponse;
use sqlx::PgPool;

use super::client_secret::verify_client_secret_async;
use super::jwt::{encode_id_token, validate_access_token, OidcIdTokenClaims};
use super::AuthState;
use crate::config::Config;

/// `authorization_code` 默认有效期（秒）。短时效 + 单次使用，降低重放窗口。
const CODE_TTL_SECS: u64 = 60;

/// 单次授权码记录（内存存储，进程内共享于 authorize/token 两端点）。
#[derive(Debug, Clone)]
pub struct OidcCodeRecord {
    /// 用户标识（来自 SSO 会话 token 的 sub）
    pub sub: String,
    /// 关联的 client_id
    pub client_id: String,
    /// 授权时校验通过的 redirect_uri（token 兑换须一致，防置换）
    pub redirect_uri: String,
    /// 授权请求携带的 nonce，原样写入 id_token
    pub nonce: Option<String>,
    /// 过期时刻（Instant，进程时钟）
    pub expires_at: Instant,
}

/// 进程内授权码存储。单实例基线实现；多实例部署应前置共享存储（如 Redis）。
#[derive(Default)]
pub struct OidcCodeStore {
    inner: Mutex<HashMap<String, OidcCodeRecord>>,
}

impl OidcCodeStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// 签发授权码并写入存储，返回码本身（随机、不可猜测）。
    pub fn issue(&self, record: OidcCodeRecord) -> String {
        let code = uuid::Uuid::new_v4().to_string();
        self.inner.lock().unwrap().insert(code.clone(), record);
        code
    }

    /// 消费授权码（单次使用）：取出并删除；过期或不存在返回 None。
    pub fn consume(&self, code: &str) -> Option<OidcCodeRecord> {
        let mut map = self.inner.lock().unwrap();
        let rec = map.remove(code)?;
        if rec.expires_at < Instant::now() {
            return None;
        }
        Some(rec)
    }
}

/// 全局单例（进程级），避免改动 Gateway 挂载契约时遗漏 app_data 注入。
static CODE_STORE: OnceLock<OidcCodeStore> = OnceLock::new();

/// 获取进程内授权码存储单例。
pub fn code_store() -> &'static OidcCodeStore {
    CODE_STORE.get_or_init(OidcCodeStore::new)
}

/// 授权端点错误。
#[derive(Debug, thiserror::Error)]
pub enum AuthorizeError {
    #[error("unsupported response_type")]
    UnsupportedResponseType,
    #[error("redirect_uri is not in the allowlist")]
    RedirectNotAllowed,
    #[error("client_id mismatch")]
    ClientMismatch,
}

/// Token 兑换错误。
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("unsupported grant_type")]
    UnsupportedGrantType,
    #[error("invalid or expired authorization_code")]
    InvalidGrant,
    #[error("invalid client authentication")]
    InvalidClient,
}

/// OIDC Token 响应（RP 换取结果）。
#[derive(Debug, Serialize)]
pub struct OidcTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub id_token: String,
}

/// `redirect_uri` 白名单校验：空白名单 → 拒绝一切；否则精确匹配。
pub fn validate_redirect_uri(allowlist: &[String], uri: &str) -> bool {
    if allowlist.is_empty() {
        return false;
    }
    allowlist.iter().any(|allowed| allowed == uri)
}

/// client_id 校验：未配置（单租户宽松）→ 放行；已配置 → 须相等。
pub fn validate_client_id(configured: &Option<String>, provided: &str) -> bool {
    match configured {
        None => true,
        Some(expected) => expected == provided,
    }
}

/// 从 DB 加载客户端并合并 config + DB 的 redirect_uris
async fn load_merged_redirect_uris(
    pool: &sqlx::PgPool,
    client_id: &str,
    config_uris: &[String],
) -> Vec<String> {
    let mut merged: Vec<String> = config_uris.to_vec();
    if let Ok(Some(uris_str)) = sqlx::query_scalar::<_, String>(
        r#"SELECT array_to_string(redirect_uris, ',') FROM isahl_auth.oidc_clients
           WHERE client_id = $1 AND enabled = TRUE AND deleted_at IS NULL"#,
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await
    {
        merged.extend(uris_str.split(',').map(|s| s.to_string()));
    }
    merged.sort();
    merged.dedup();
    merged
}

/// DB 多客户端 client_id 校验：config 未配置或匹配 → true；否则查 DB
async fn validate_client_id_multi(
    pool: &sqlx::PgPool,
    config_client_id: &Option<String>,
    provided: &str,
) -> bool {
    if validate_client_id(config_client_id, provided) {
        return true;
    }
    // 查 DB 中是否存在已启用的客户端
    sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM isahl_auth.oidc_clients WHERE client_id = $1 AND enabled = TRUE AND deleted_at IS NULL",
    )
    .bind(provided)
    .fetch_optional(pool)
    .await
    .map(|r| r.is_some())
    .unwrap_or(false)
}

/// 构造授权重定向 URL（`redirect_uri?code=...&state=...`）。
fn build_redirect_url(redirect_uri: &str, code: &str, state: Option<&str>) -> String {
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    match state {
        Some(s) if !s.is_empty() => format!("{}{}code={}&state={}", redirect_uri, sep, code, s),
        _ => format!("{}{}code={}", redirect_uri, sep, code),
    }
}

/// `/oidc/authorize` 请求参数（从查询字符串归一化后的结构化输入）。
#[derive(Debug, Clone)]
pub struct OidcAuthorizeInput {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub nonce: Option<String>,
}

/// 授权逻辑（纯函数，便于测试）：校验参数 → 签发 code → 返回重定向 URL。
pub fn build_authorize_redirect(
    store: &OidcCodeStore,
    config: &Config,
    input: &OidcAuthorizeInput,
    sub: &str,
) -> Result<String, AuthorizeError> {
    if input.response_type != "code" {
        return Err(AuthorizeError::UnsupportedResponseType);
    }
    if !validate_redirect_uri(&config.oidc_redirect_uris, &input.redirect_uri) {
        return Err(AuthorizeError::RedirectNotAllowed);
    }
    if !validate_client_id(&config.oidc_client_id, &input.client_id) {
        return Err(AuthorizeError::ClientMismatch);
    }
    let record = OidcCodeRecord {
        sub: sub.to_string(),
        client_id: input.client_id.clone(),
        redirect_uri: input.redirect_uri.clone(),
        nonce: input.nonce.clone(),
        expires_at: Instant::now() + Duration::from_secs(CODE_TTL_SECS),
    };
    let code = store.issue(record);
    Ok(build_redirect_url(
        &input.redirect_uri,
        &code,
        input.state.as_deref(),
    ))
}

/// Token 兑换逻辑（纯函数，便于测试）：消费 code → 校验绑定 → 签发 id_token。
pub fn exchange_code_for_id_token(
    store: &OidcCodeStore,
    config: &Config,
    grant_type: &str,
    code: &str,
    client_id: &str,
    redirect_uri: &str,
    private_key: &[u8],
) -> Result<OidcTokenResponse, TokenError> {
    if grant_type != "authorization_code" {
        return Err(TokenError::UnsupportedGrantType);
    }
    let record = store.consume(code).ok_or(TokenError::InvalidGrant)?;
    if record.client_id != client_id || record.redirect_uri != redirect_uri {
        return Err(TokenError::InvalidGrant);
    }

    let now = Utc::now().timestamp() as usize;
    let claims = OidcIdTokenClaims {
        iss: config.oidc_issuer.clone(),
        sub: record.sub.clone(),
        aud: client_id.to_string(),
        iat: now,
        exp: (now + CODE_TTL_SECS as usize),
        nonce: record.nonce.clone(),
    };
    let id_token = encode_id_token(&claims, private_key).map_err(|_| TokenError::InvalidGrant)?;

    Ok(OidcTokenResponse {
        access_token: format!("oidc-{}", uuid::Uuid::new_v4()),
        token_type: "Bearer".to_string(),
        id_token,
    })
}

/// Token 端点 client_secret 校验：按 `client_id` 查询 `isahl_auth.oidc_clients` 的
/// `client_secret_hash` 并校验 `client_secret`。
///
/// 行为：
/// - 无匹配客户端行 → 放行（依赖授权码内绑定的 client_id 校验，见 `exchange_code_for_id_token`）。
/// - 匹配行但 `client_secret_hash` 为空 → 放行（public client / 历史无 secret 记录）。
/// - 匹配行且 `client_secret_hash` 非空 → 必须校验通过，否则返回 `InvalidClient`。
///
/// 散列算法详见 `client_secret.rs`：argon2id 为权威格式，遗留 MD5（32 位十六进制）仍受支持。
pub async fn verify_client_secret_for_token(
    pool: &PgPool,
    client_id: &str,
    client_secret: &str,
) -> Result<(), TokenError> {
    let stored: Option<String> = sqlx::query_scalar(
        r#"SELECT client_secret_hash FROM isahl_auth.oidc_clients
           WHERE client_id = $1 AND enabled = TRUE AND deleted_at IS NULL"#,
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        log::error!("Failed to load oidc client secret for {}: {}", client_id, e);
        TokenError::InvalidClient
    })?;

    match stored {
        None => Ok(()),
        Some(hash) if hash.is_empty() => Ok(()),
        Some(hash) => match verify_client_secret_async(client_secret.to_string(), hash).await {
            Ok(_) => Ok(()),
            Err(_) => Err(TokenError::InvalidClient),
        },
    }
}

/// 构造 OIDC Discovery 文档（`.well-known/openid-configuration`）。
pub fn build_discovery_document(issuer: &str) -> serde_json::Value {
    let base = issuer.trim_end_matches('/');
    serde_json::json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{}/oidc/authorize", base),
        "token_endpoint": format!("{}/oidc/token", base),
        "jwks_uri": format!("{}/.well-known/jwks.json", base),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["ES256"],
        "scopes_supported": ["openid"],
        "claims_supported": ["iss", "sub", "aud", "exp", "iat", "nonce"],
    })
}

/// GET /.well-known/openid-configuration
pub async fn oidc_discovery(cfg: web::Data<Config>) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("application/json")
        .json(build_discovery_document(&cfg.oidc_issuer))
}

/// 授权请求查询参数。
#[derive(Debug, Deserialize)]
pub struct OidcAuthorizeQuery {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub scope: Option<String>,
}

/// GET /oidc/authorize
///
/// 要求调用方已持有 SSO 会话（access_token）。未认证返回 401；参数或白名单校验失败返回 400。
pub async fn oidc_authorize(
    req: HttpRequest,
    query: web::Query<OidcAuthorizeQuery>,
    cfg: web::Data<Config>,
    auth_state: web::Data<AuthState>,
    pool: web::Data<sqlx::PgPool>,
) -> HttpResponse {
    let claims = match validate_access_token(&req, &auth_state.verification_keys()).await {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse::unauthorized(
                "OIDC authorize requires an authenticated SSO session",
            ));
        }
    };

    let input = OidcAuthorizeInput {
        response_type: query.response_type.clone().unwrap_or_default(),
        client_id: query.client_id.clone().unwrap_or_default(),
        redirect_uri: query.redirect_uri.clone().unwrap_or_default(),
        state: query.state.clone(),
        nonce: query.nonce.clone(),
    };

    // Multi-client validation: check config + DB
    if !validate_client_id_multi(pool.get_ref(), &cfg.oidc_client_id, &input.client_id).await {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "CLIENT_MISMATCH",
            "client_id is not accepted",
        ));
    }

    // Load merged redirect_uris from config + DB Clients
    let merged_uris =
        load_merged_redirect_uris(pool.get_ref(), &input.client_id, &cfg.oidc_redirect_uris).await;

    if !validate_redirect_uri(&merged_uris, &input.redirect_uri) {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "REDIRECT_NOT_ALLOWED",
            "redirect_uri is not in the allowlist",
        ));
    }

    if input.response_type != "code" {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "UNSUPPORTED_RESPONSE_TYPE",
            "only response_type=code is supported",
        ));
    }

    // Issue authorization code
    let record = OidcCodeRecord {
        sub: claims.sub.clone(),
        client_id: input.client_id.clone(),
        redirect_uri: input.redirect_uri.clone(),
        nonce: input.nonce.clone(),
        expires_at: Instant::now() + Duration::from_secs(CODE_TTL_SECS),
    };
    let code = code_store().issue(record);
    let location = build_redirect_url(&input.redirect_uri, &code, input.state.as_deref());

    HttpResponse::Found()
        .insert_header((header::LOCATION, location))
        .finish()
}

/// Token 兑换请求体（application/x-www-form-urlencoded）。
#[derive(Debug, Deserialize)]
pub struct OidcTokenRequest {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

/// POST /oidc/token
pub async fn oidc_token(
    form: web::Form<OidcTokenRequest>,
    cfg: web::Data<Config>,
    auth_state: web::Data<AuthState>,
    pool: web::Data<PgPool>,
) -> HttpResponse {
    let grant_type = form.grant_type.as_deref().unwrap_or("");
    let code = form.code.as_deref().unwrap_or("");
    let client_id = form.client_id.as_deref().unwrap_or("");
    let redirect_uri = form.redirect_uri.as_deref().unwrap_or("");
    let client_secret = form.client_secret.as_deref().unwrap_or("");

    // 1. client_secret 校验（无记录 / 空 secret 时跳过，兼容 public client）
    if let Err(TokenError::InvalidClient) =
        verify_client_secret_for_token(pool.get_ref(), client_id, client_secret).await
    {
        return HttpResponse::Unauthorized().json(ErrorResponse::new(
            "INVALID_CLIENT",
            "client authentication failed",
        ));
    }

    match exchange_code_for_id_token(
        code_store(),
        &cfg,
        grant_type,
        code,
        client_id,
        redirect_uri,
        &auth_state.jwt_private_key,
    ) {
        Ok(resp) => HttpResponse::Ok()
            .content_type("application/json")
            .json(resp),
        Err(TokenError::UnsupportedGrantType) => {
            HttpResponse::BadRequest().json(ErrorResponse::new(
                "UNSUPPORTED_GRANT_TYPE",
                "only grant_type=authorization_code is supported",
            ))
        }
        Err(TokenError::InvalidGrant) => HttpResponse::BadRequest().json(ErrorResponse::new(
            "INVALID_GRANT",
            "invalid, expired, or already-used authorization_code",
        )),
        Err(TokenError::InvalidClient) => HttpResponse::Unauthorized().json(ErrorResponse::new(
            "INVALID_CLIENT",
            "client authentication failed",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 测试用 EC P-256 私钥（PKCS#8 PEM）
    const TEST_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgkUXkbsFeGTxRAuvY
\
FRUoGkTqPKKLYJ+1c5A4sQbKDlChRANCAAQujoQbf1/KuEBy50ws3vLxtBszp+wj
\
t3ac6CVz6zQl8Vb7sTqje0wGgbP8auaIsYof1dX4B6PM2FglnfMScRaP
\
-----END PRIVATE KEY-----";

    const TEST_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAELo6EG39fyrhAcudMLN7y8bQbM6fs
\
I7d2nOglc+s0JfFW+7E6o3tMBoGz/GrmiLGKH9XV+AejzNhYJZ3zEnEWjw==
\
-----END PUBLIC KEY-----";

    fn test_config(allowlist: &[&str], client_id: Option<&str>) -> Config {
        Config {
            server_addr: "0.0.0.0:9002".into(),
            database_url: "postgres://localhost:5432/test".into(),
            sso_jwt_private_key: String::new(),
            encryption_key: "k".into(),
            ngac_preview_dir: None,
            jwt_access_expiry: 900,
            jwt_refresh_expiry: 604800,
            oauth_google_client_id: None,
            oauth_google_client_secret: None,
            oauth_github_client_id: None,
            oauth_github_client_secret: None,
            oauth_microsoft_client_id: None,
            oauth_microsoft_client_secret: None,
            oauth_microsoft_tenant_id: None,
            oauth_okta_domain: None,
            oauth_okta_client_id: None,
            oauth_okta_client_secret: None,
            oauth_redirect_url: "http://localhost:9002/auth/callback".into(),
            oidc_issuer: "http://localhost:9002".into(),
            oidc_client_id: client_id.map(|s| s.to_string()),
            oidc_redirect_uris: allowlist.iter().map(|s| s.to_string()).collect(),
            log_level: "info".into(),
            identity_verify_mode: "local".into(),
            identity_external_verify_url: None,
            email_mode: "smtp".into(),
            sso_jwt_public_key_prev: None,
        }
    }

    #[test]
    fn test_validate_redirect_uri_allowlist() {
        let allow = vec!["https://rp.example.com/cb".to_string()];
        assert!(validate_redirect_uri(&allow, "https://rp.example.com/cb"));
        assert!(!validate_redirect_uri(
            &allow,
            "https://evil.example.com/cb"
        ));
        // 空白名单拒绝一切，防未配置时开放重定向
        assert!(!validate_redirect_uri(&[], "https://rp.example.com/cb"));
    }

    #[test]
    fn test_validate_client_id() {
        assert!(validate_client_id(&None, "anything"));
        assert!(validate_client_id(&Some("myclient".into()), "myclient"));
        assert!(!validate_client_id(&Some("myclient".into()), "other"));
    }

    #[test]
    fn test_discovery_document_contains_es256() {
        let doc = build_discovery_document("http://localhost:9002");
        assert_eq!(doc["issuer"], "http://localhost:9002");
        assert_eq!(
            doc["authorization_endpoint"],
            "http://localhost:9002/oidc/authorize"
        );
        assert_eq!(doc["token_endpoint"], "http://localhost:9002/oidc/token");
        assert_eq!(
            doc["jwks_uri"],
            "http://localhost:9002/.well-known/jwks.json"
        );
        let algs = doc["id_token_signing_alg_values_supported"]
            .as_array()
            .unwrap();
        assert!(algs.iter().any(|v| v == "ES256"));
    }

    fn auth_input(
        response_type: &str,
        client_id: &str,
        redirect_uri: &str,
        state: Option<&str>,
        nonce: Option<&str>,
    ) -> OidcAuthorizeInput {
        OidcAuthorizeInput {
            response_type: response_type.to_string(),
            client_id: client_id.to_string(),
            redirect_uri: redirect_uri.to_string(),
            state: state.map(|s| s.to_string()),
            nonce: nonce.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_authorize_rejects_unwhitelisted_redirect() {
        let store = OidcCodeStore::new();
        let cfg = test_config(&["https://rp.example.com/cb"], None);
        let res = build_authorize_redirect(
            &store,
            &cfg,
            &auth_input(
                "code",
                "client",
                "https://evil.example.com/cb",
                Some("st"),
                None,
            ),
            "user1",
        );
        assert!(matches!(res, Err(AuthorizeError::RedirectNotAllowed)));
    }

    #[test]
    fn test_authorize_rejects_bad_response_type() {
        let store = OidcCodeStore::new();
        let cfg = test_config(&["https://rp.example.com/cb"], None);
        let res = build_authorize_redirect(
            &store,
            &cfg,
            &auth_input(
                "token",
                "client",
                "https://rp.example.com/cb",
                Some("st"),
                None,
            ),
            "user1",
        );
        assert!(matches!(res, Err(AuthorizeError::UnsupportedResponseType)));
    }

    #[test]
    fn test_authorize_issues_code_and_redirects_with_state() {
        let store = OidcCodeStore::new();
        let cfg = test_config(&["https://rp.example.com/cb"], None);
        let res = build_authorize_redirect(
            &store,
            &cfg,
            &auth_input(
                "code",
                "client",
                "https://rp.example.com/cb",
                Some("xyz"),
                Some("nonce123"),
            ),
            "user42",
        );
        let url = res.expect("authorize should succeed");
        assert!(url.starts_with("https://rp.example.com/cb?code="));
        assert!(url.contains("state=xyz"));
        // 存储中应恰好一条记录
        assert_eq!(store.inner.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_token_exchange_issues_verifiable_id_token() {
        let store = OidcCodeStore::new();
        let cfg = test_config(&["https://rp.example.com/cb"], Some("myclient"));
        let url = build_authorize_redirect(
            &store,
            &cfg,
            &auth_input(
                "code",
                "myclient",
                "https://rp.example.com/cb",
                Some("st"),
                Some("nonce123"),
            ),
            "user42",
        )
        .unwrap();
        let code = url
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();

        let resp = exchange_code_for_id_token(
            &store,
            &cfg,
            "authorization_code",
            &code,
            "myclient",
            "https://rp.example.com/cb",
            TEST_PRIVATE_KEY,
        )
        .expect("exchange should succeed");

        assert_eq!(resp.token_type, "Bearer");
        // 用 JWKS 派生公钥验签 id_token
        let kid = super::super::jwt::public_key_kid(TEST_PUBLIC_KEY).unwrap();
        let jwk = super::super::jwt::public_key_to_jwk(TEST_PUBLIC_KEY, &kid).unwrap();
        let x = jwk["x"].as_str().unwrap();
        let y = jwk["y"].as_str().unwrap();
        let key = jsonwebtoken::DecodingKey::from_ec_components(x, y).unwrap();
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.validate_exp = true;
        validation.set_audience(&["myclient"]);
        let decoded =
            jsonwebtoken::decode::<OidcIdTokenClaims>(&resp.id_token, &key, &validation).unwrap();
        assert_eq!(decoded.claims.iss, "http://localhost:9002");
        assert_eq!(decoded.claims.sub, "user42");
        assert_eq!(decoded.claims.aud, "myclient");
        assert_eq!(decoded.claims.nonce.as_deref(), Some("nonce123"));
    }

    #[test]
    fn test_code_is_single_use_and_replay_rejected() {
        let store = OidcCodeStore::new();
        let cfg = test_config(&["https://rp.example.com/cb"], None);
        let url = build_authorize_redirect(
            &store,
            &cfg,
            &auth_input("code", "client", "https://rp.example.com/cb", None, None),
            "u1",
        )
        .unwrap();
        let code = url
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();

        let first = exchange_code_for_id_token(
            &store,
            &cfg,
            "authorization_code",
            &code,
            "client",
            "https://rp.example.com/cb",
            TEST_PRIVATE_KEY,
        );
        assert!(first.is_ok());
        // 第二次兑换同一 code → 已被消费，拒绝
        let second = exchange_code_for_id_token(
            &store,
            &cfg,
            "authorization_code",
            &code,
            "client",
            "https://rp.example.com/cb",
            TEST_PRIVATE_KEY,
        );
        assert!(matches!(second, Err(TokenError::InvalidGrant)));
    }

    #[test]
    fn test_exchange_rejects_wrong_client_or_redirect() {
        let store = OidcCodeStore::new();
        let cfg = test_config(&["https://rp.example.com/cb"], None);
        let url = build_authorize_redirect(
            &store,
            &cfg,
            &auth_input("code", "client", "https://rp.example.com/cb", None, None),
            "u1",
        )
        .unwrap();
        let code = url
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();

        let wrong_client = exchange_code_for_id_token(
            &store,
            &cfg,
            "authorization_code",
            &code,
            "other",
            "https://rp.example.com/cb",
            TEST_PRIVATE_KEY,
        );
        assert!(matches!(wrong_client, Err(TokenError::InvalidGrant)));
    }

    // ── client_secret 校验集成测试 ──────────────────────────────────────────────
    // 测试库默认不含 isahl_auth.oidc_clients，需要时在测试内自建并清理。
    use crate::auth::client_secret::hash_client_secret;

    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
            .unwrap_or_else(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "william.d.zk".to_string());
                format!("postgres://{}@localhost:5432/aliothstudio_test", user)
            });
        sqlx::PgPool::connect(&url)
            .await
            .expect("无法连接测试库，请先运行 `bash scripts/db/reset-db.sh --test`")
    }

    async fn ensure_oidc_clients_table(pool: &sqlx::PgPool) {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS isahl_auth.oidc_clients (
                id BIGSERIAL PRIMARY KEY,
                client_id VARCHAR(128) NOT NULL UNIQUE,
                client_name VARCHAR(256) NOT NULL DEFAULT '',
                client_secret_hash VARCHAR(256) NOT NULL DEFAULT '',
                redirect_uris TEXT[] NOT NULL DEFAULT '{}',
                enabled BOOLEAN NOT NULL DEFAULT TRUE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                deleted_at TIMESTAMPTZ
            )"#,
        )
        .execute(pool)
        .await
        .expect("创建 oidc_clients 测试表失败");
    }

    async fn seed_client(pool: &sqlx::PgPool, client_id: &str, secret_hash: &str) {
        sqlx::query(
            "INSERT INTO isahl_auth.oidc_clients (client_id, client_secret_hash, enabled) \
             VALUES ($1, $2, TRUE) ON CONFLICT (client_id) DO UPDATE SET client_secret_hash = $2, deleted_at = NULL",
        )
        .bind(client_id)
        .bind(secret_hash)
        .execute(pool)
        .await
        .expect("seed oidc client 失败");
    }

    async fn cleanup_client(pool: &sqlx::PgPool, client_id: &str) {
        let _ = sqlx::query("DELETE FROM isahl_auth.oidc_clients WHERE client_id = $1")
            .bind(client_id)
            .execute(pool)
            .await;
    }

    #[tokio::test]
    async fn verify_client_secret_for_token_matrix() {
        let pool = test_pool().await;
        ensure_oidc_clients_table(&pool).await;

        // 遗留 MD5（32 位十六进制）
        let legacy_hash = format!("{:x}", md5::compute(b"legacypass"));
        // 新标准 argon2id
        let argon_hash = hash_client_secret("argonpass").unwrap();

        seed_client(&pool, "cs_legacy", &legacy_hash).await;
        seed_client(&pool, "cs_argon", &argon_hash).await;
        seed_client(&pool, "cs_public", "").await;

        // 1. 正确 legacy MD5 → 放行
        assert!(
            verify_client_secret_for_token(&pool, "cs_legacy", "legacypass")
                .await
                .is_ok()
        );
        // 2. 错误 legacy MD5 → 拒绝
        assert!(matches!(
            verify_client_secret_for_token(&pool, "cs_legacy", "wrong").await,
            Err(TokenError::InvalidClient)
        ));
        // 3. 正确 argon2id → 放行
        assert!(
            verify_client_secret_for_token(&pool, "cs_argon", "argonpass")
                .await
                .is_ok()
        );
        // 4. 错误 argon2id → 拒绝
        assert!(matches!(
            verify_client_secret_for_token(&pool, "cs_argon", "wrong").await,
            Err(TokenError::InvalidClient)
        ));
        // 5. public client（空 secret）→ 放行，不校验
        assert!(verify_client_secret_for_token(&pool, "cs_public", "")
            .await
            .is_ok());
        assert!(
            verify_client_secret_for_token(&pool, "cs_public", "ignored")
                .await
                .is_ok()
        );
        // 6. 未知 client_id → 放行（依赖授权码绑定的 client_id 校验）
        assert!(verify_client_secret_for_token(&pool, "cs_unknown", "x")
            .await
            .is_ok());

        cleanup_client(&pool, "cs_legacy").await;
        cleanup_client(&pool, "cs_argon").await;
        cleanup_client(&pool, "cs_public").await;
    }
}
