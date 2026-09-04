//! OAuth2 客户端实现
//!
//! 提供 OAuth2 Authorization Code 流程支持，包括：
//! - 授权 URL 生成
//! - PKCE (Proof Key for Code Exchange) 支持
//! - State 参数生成和验证
//! - 令牌交换

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{distr::Alphanumeric, RngExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// OAuth2 授权请求参数
#[derive(Debug, Serialize)]
pub struct AuthorizationRequest {
    /// 客户端 ID
    pub client_id: String,
    /// 响应类型 (固定为 "code")
    pub response_type: String,
    /// 重定向 URI
    pub redirect_uri: String,
    /// 作用域
    pub scope: String,
    /// State 参数 (用于 CSRF 防护)
    pub state: String,
    /// PKCE Code Challenge
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_challenge: Option<String>,
    /// PKCE Code Challenge Method
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_challenge_method: Option<String>,
    /// 可选：登录提示 (用于强制选择账户)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl AuthorizationRequest {
    /// 创建新的授权请求
    pub fn new(
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
        scope: impl Into<String>,
        state: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            response_type: "code".to_string(),
            redirect_uri: redirect_uri.into(),
            scope: scope.into(),
            state: state.into(),
            code_challenge: None,
            code_challenge_method: None,
            prompt: None,
        }
    }

    /// 添加 PKCE 支持
    pub fn with_pkce(mut self, code_challenge: impl Into<String>) -> Self {
        self.code_challenge = Some(code_challenge.into());
        self.code_challenge_method = Some("S256".to_string());
        self
    }

    /// 转换为查询字符串
    pub fn to_query_string(&self) -> String {
        let mut params = vec![
            format!("client_id={}", urlencoding::encode(&self.client_id)),
            format!("response_type={}", urlencoding::encode(&self.response_type)),
            format!("redirect_uri={}", urlencoding::encode(&self.redirect_uri)),
            format!("scope={}", urlencoding::encode(&self.scope)),
            format!("state={}", urlencoding::encode(&self.state)),
        ];

        if let Some(ref challenge) = self.code_challenge {
            params.push(format!("code_challenge={}", urlencoding::encode(challenge)));
        }
        if let Some(ref method) = self.code_challenge_method {
            params.push(format!(
                "code_challenge_method={}",
                urlencoding::encode(method)
            ));
        }
        if let Some(ref prompt) = self.prompt {
            params.push(format!("prompt={}", urlencoding::encode(prompt)));
        }

        params.join("&")
    }
}

/// OAuth2 令牌请求 (Authorization Code 交换)
#[derive(Debug, Serialize)]
pub struct TokenRequest {
    /// 授权类型 (固定为 "authorization_code")
    pub grant_type: String,
    /// 客户端 ID
    pub client_id: String,
    /// 客户端密钥
    pub client_secret: String,
    /// 授权码
    pub code: String,
    /// 重定向 URI (必须与授权请求一致)
    pub redirect_uri: String,
    /// PKCE Code Verifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_verifier: Option<String>,
}

impl TokenRequest {
    /// 创建新的令牌请求
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        code: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            grant_type: "authorization_code".to_string(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            code: code.into(),
            redirect_uri: redirect_uri.into(),
            code_verifier: None,
        }
    }

    /// 添加 PKCE code verifier
    pub fn with_code_verifier(mut self, verifier: impl Into<String>) -> Self {
        self.code_verifier = Some(verifier.into());
        self
    }
}

/// OAuth2 令牌响应
#[derive(Debug, Deserialize, Clone)]
pub struct TokenResponse {
    /// 访问令牌
    pub access_token: String,
    /// 令牌类型 (通常为 "Bearer")
    pub token_type: String,
    /// 过期时间 (秒)
    #[serde(with = "common::serde_zuid::opt", default)]
    pub expires_in: Option<i64>,
    /// 刷新令牌
    pub refresh_token: Option<String>,
    /// 作用域
    pub scope: Option<String>,
    /// OIDC ID Token
    pub id_token: Option<String>,
}

/// OAuth2 错误响应
#[derive(Debug, Deserialize)]
pub struct OAuthError {
    pub error: String,
    #[serde(default)]
    pub error_description: Option<String>,
    #[serde(default)]
    pub error_uri: Option<String>,
}

/// PKCE Code Verifier
#[derive(Debug, Clone)]
pub struct PkceVerifier {
    /// 原始 verifier (43-128 字符)
    pub verifier: String,
}

impl PkceVerifier {
    /// 生成新的 PKCE verifier
    pub fn generate() -> Self {
        // 生成 128 字符的随机字符串
        let verifier: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(128)
            .map(char::from)
            .collect();

        Self { verifier }
    }

    /// 计算 S256 code challenge
    pub fn challenge_s256(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.verifier);
        let hash = hasher.finalize();
        URL_SAFE_NO_PAD.encode(hash)
    }

    /// 获取 verifier 字符串
    pub fn as_str(&self) -> &str {
        &self.verifier
    }
}

/// 生成随机 state 参数
pub fn generate_state() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

/// OAuth2 客户端
pub struct OAuth2Client {
    client_id: String,
    client_secret: String,
    authorization_endpoint: String,
    token_endpoint: String,
    redirect_uri: String,
}

impl OAuth2Client {
    /// 创建新的 OAuth2 客户端
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        authorization_endpoint: impl Into<String>,
        token_endpoint: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            authorization_endpoint: authorization_endpoint.into(),
            token_endpoint: token_endpoint.into(),
            redirect_uri: redirect_uri.into(),
        }
    }

    /// 生成授权 URL (带 PKCE)
    pub fn authorize_url(&self, scope: &str, state: &str, pkce_verifier: &PkceVerifier) -> String {
        let challenge = pkce_verifier.challenge_s256();
        let request = AuthorizationRequest::new(&self.client_id, &self.redirect_uri, scope, state)
            .with_pkce(challenge);

        format!(
            "{}?{}",
            self.authorization_endpoint,
            request.to_query_string()
        )
    }

    /// 生成授权 URL (不带 PKCE)
    pub fn authorize_url_without_pkce(&self, scope: &str, state: &str) -> String {
        let request = AuthorizationRequest::new(&self.client_id, &self.redirect_uri, scope, state);

        format!(
            "{}?{}",
            self.authorization_endpoint,
            request.to_query_string()
        )
    }

    /// 交换授权码获取令牌
    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: Option<&str>,
    ) -> Result<TokenResponse, OAuth2Error> {
        let client = crate::http_client::get().clone();

        let mut request = TokenRequest::new(
            &self.client_id,
            &self.client_secret,
            code,
            &self.redirect_uri,
        );

        if let Some(verifier) = code_verifier {
            request = request.with_code_verifier(verifier);
        }

        let response = client
            .post(&self.token_endpoint)
            .form(&request)
            .send()
            .await
            .map_err(|e| OAuth2Error::HttpError(e.to_string()))?;

        if response.status().is_success() {
            let token_response = response
                .json::<TokenResponse>()
                .await
                .map_err(|e| OAuth2Error::ParseError(e.to_string()))?;
            Ok(token_response)
        } else {
            let error = response
                .json::<OAuthError>()
                .await
                .map_err(|e| OAuth2Error::ParseError(e.to_string()))?;
            Err(OAuth2Error::OAuthError(error))
        }
    }
}

/// OAuth2 错误类型
#[derive(Debug, thiserror::Error)]
pub enum OAuth2Error {
    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("OAuth error: {0:?}")]
    OAuthError(OAuthError),

    #[error("Invalid state")]
    InvalidState,

    #[error("PKCE verification failed")]
    PkceVerificationFailed,
}

use actix_web::{web, HttpResponse};

/// 支持的 OAuth Provider 信息
#[derive(Debug, serde::Serialize)]
pub struct OAuthProviderInfo {
    pub id: String,
    pub name: String,
    pub authorization_endpoint: String,
}

/// 列出已配置的 OAuth Provider（DB 驱动：isahl_auth.identity_providers 启用项；
/// 未配置返回空数组——登录页按此隐藏社交登录区，修复"Unknown provider: github"）
async fn list_oauth_providers(pool: web::Data<sqlx::PgPool>) -> HttpResponse {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT provider_type, name, COALESCE(authorization_endpoint, '') \
         FROM isahl_auth.identity_providers \
         WHERE enabled AND deleted_at IS NULL ORDER BY id",
    )
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();
    let providers: Vec<OAuthProviderInfo> = rows
        .into_iter()
        .map(|(id, name, endpoint)| OAuthProviderInfo {
            id,
            name,
            authorization_endpoint: endpoint,
        })
        .collect();
    HttpResponse::Ok().json(providers)
}

/// 配置 OAuth 路由
/// 配置 OAuth 路由
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/oauth")
            .route("/providers", web::get().to(list_oauth_providers))
            .route("/login", web::post().to(super::oauth_callback::oauth_login))
            .route(
                "/callback",
                web::get().to(super::oauth_callback::oauth_callback),
            ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_verifier_generation() {
        let verifier = PkceVerifier::generate();
        assert_eq!(verifier.verifier.len(), 128);

        let challenge = verifier.challenge_s256();
        // S256 hash is 32 bytes, base64url encoded is 43 chars
        assert_eq!(challenge.len(), 43);
    }

    #[test]
    fn test_state_generation() {
        let state1 = generate_state();
        let state2 = generate_state();

        assert_eq!(state1.len(), 32);
        assert_eq!(state2.len(), 32);
        assert_ne!(state1, state2); // Should be random
    }

    #[test]
    fn test_authorization_request_to_query_string() {
        let request = AuthorizationRequest::new(
            "client123",
            "https://example.com/callback",
            "openid email",
            "state123",
        );

        let query = request.to_query_string();
        assert!(query.contains("client_id=client123"));
        assert!(query.contains("response_type=code"));
        assert!(query.contains("state=state123"));
        assert!(query.contains("scope=openid%20email"));
    }

    #[test]
    fn test_authorization_request_with_pkce() {
        let request = AuthorizationRequest::new(
            "client123",
            "https://example.com/callback",
            "openid email",
            "state123",
        )
        .with_pkce("challenge123");

        let query = request.to_query_string();
        assert!(query.contains("code_challenge=challenge123"));
        assert!(query.contains("code_challenge_method=S256"));
    }
}
