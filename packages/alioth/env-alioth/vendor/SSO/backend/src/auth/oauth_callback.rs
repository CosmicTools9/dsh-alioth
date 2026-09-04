//! OAuth 回调处理
//!
//! 处理 OAuth2/OIDC 回调流程：
//! - 验证 state 参数
//! - 交换授权码获取令牌
//! - 获取用户信息
//! - 创建或绑定用户
//! - 颁发 JWT

use actix_web::{web, HttpResponse};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};

use crate::auth::{
    jwt::{encode_access_token, encode_refresh_token, set_refresh_cookie, Claims},
    oauth::{OAuth2Client, PkceVerifier, TokenResponse},
    oidc::{extract_user_info_from_userinfo, NormalizedUserInfo},
};

/// OAuth 回调请求查询参数
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    /// 授权码
    pub code: String,
    /// State 参数
    pub state: String,
    /// 错误代码 (如果授权失败)
    pub error: Option<String>,
    /// 错误描述
    pub error_description: Option<String>,
}

/// OAuth 登录 URL 请求
#[derive(Debug, Deserialize)]
pub struct OAuthLoginRequest {
    /// 身份提供商名称 (google, github, microsoft)
    pub provider: String,
    /// 登录成功后重定向 URL (可选)
    pub redirect_url: Option<String>,
}

/// OAuth 登录 URL 响应
#[derive(Debug, Serialize)]
pub struct OAuthLoginResponse {
    /// 授权 URL
    pub auth_url: String,
    /// State 参数 (用于前端验证)
    pub state: String,
}

/// OAuth 错误响应
#[derive(Debug, Serialize)]
pub struct OAuthErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 用户信息 (从数据库查询)
#[derive(Debug, Clone)]
struct UserInfo {
    id: String,
    email: Option<String>,
    _mfa_enabled: bool,
}

/// 身份提供商配置
#[derive(Debug, Clone)]
struct ProviderConfig {
    id: i64,
    name: String,
    provider_type: String,
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: Option<String>,
    _jwks_uri: Option<String>,
    client_id: String,
    client_secret: String,
    scopes: String,
    field_mapping: serde_json::Value,
}

/// OAuth 状态信息
struct OAuthStateInfo {
    #[allow(dead_code)]
    id: i64,
    provider_id: i64,
    pkce_verifier: String,
    #[allow(dead_code)]
    user_id: Option<String>,
    redirect_url: String,
}

/// 生成 OAuth 登录 URL
pub async fn oauth_login(
    pool: web::Data<PgPool>,
    body: web::Json<OAuthLoginRequest>,
) -> HttpResponse {
    oauth_login_handler(pool, body).await
}

/// 生成 OAuth 登录 URL (内部处理函数)
async fn oauth_login_handler(
    pool: web::Data<PgPool>,
    body: web::Json<OAuthLoginRequest>,
) -> HttpResponse {
    // 获取身份提供商配置
    let provider = match get_provider_config(&pool, &body.provider).await {
        Ok(Some((p, true))) => p,
        Ok(Some((_, false))) => {
            return HttpResponse::BadRequest().json(OAuthErrorResponse {
                error: "provider_disabled".to_string(),
                message: Some(format!("Provider '{}' is disabled", body.provider)),
            });
        }
        Ok(None) => {
            return HttpResponse::BadRequest().json(OAuthErrorResponse {
                error: "unknown_provider".to_string(),
                message: Some(format!("Unknown provider: {}", body.provider)),
            });
        }
        Err(e) => {
            log::error!("Failed to get provider config: {}", e);
            return HttpResponse::InternalServerError().json(OAuthErrorResponse {
                error: "internal_error".to_string(),
                message: Some("Failed to get provider configuration".to_string()),
            });
        }
    };

    // 生成 state 和 PKCE verifier
    let state = crate::auth::oauth::generate_state();
    let pkce_verifier = PkceVerifier::generate();

    // 存储 state 到数据库
    let redirect_url = body.redirect_url.clone().unwrap_or_else(|| "/".to_string());
    if let Err(e) = store_oauth_state(
        &pool,
        &state,
        &pkce_verifier,
        &provider.id,
        None, // user_id (用于已登录用户绑定)
        &redirect_url,
    )
    .await
    {
        log::error!("Failed to store OAuth state: {}", e);
        return HttpResponse::InternalServerError().json(OAuthErrorResponse {
            error: "internal_error".to_string(),
            message: Some("Failed to initialize OAuth flow".to_string()),
        });
    }

    // 构建授权 URL
    let redirect_uri = format!("{}/auth/oauth/callback", get_base_url());
    let auth_url = match provider.name.as_str() {
        // 微信：非标准 OAuth2（appid 参数名、不支持 PKCE、网页授权需 #wechat_redirect）
        "WeChat" => build_wechat_auth_url(&provider, &redirect_uri, &state),
        _ => {
            let client = OAuth2Client::new(
                &provider.client_id,
                &provider.client_secret,
                &provider.authorization_endpoint,
                &provider.token_endpoint,
                redirect_uri,
            );
            client.authorize_url(&provider.scopes, &state, &pkce_verifier)
        }
    };

    HttpResponse::Ok().json(OAuthLoginResponse { auth_url, state })
}

/// 构造微信 OAuth2 授权 URL（扫码登录与公众号网页授权共用）。
///
/// 微信与标准 OAuth2 的差异：
/// - 参数名用 `appid` 而非 `client_id`
/// - 不支持 PKCE，不传 `code_challenge`
/// - 公众号网页授权（authorization_endpoint 含 `oauth2/authorize`）必须在 URL 末尾追加
///   `#wechat_redirect`；扫码登录（`qrconnect`）不需要
fn build_wechat_auth_url(provider: &ProviderConfig, redirect_uri: &str, state: &str) -> String {
    let mut url = format!(
        "{}?appid={}&redirect_uri={}&response_type=code&scope={}&state={}",
        provider.authorization_endpoint,
        urlencoding::encode(&provider.client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(&provider.scopes),
        urlencoding::encode(state),
    );
    if provider.authorization_endpoint.contains("oauth2/authorize") {
        url.push_str("#wechat_redirect");
    }
    url
}

/// 处理 OAuth 回调
pub async fn oauth_callback(
    pool: web::Data<PgPool>,
    state_data: web::Data<OAuthAuthState>,
    query: web::Query<OAuthCallbackQuery>,
) -> HttpResponse {
    oauth_callback_handler(pool, state_data, query).await
}

/// 处理 OAuth 回调 (内部处理函数)
async fn oauth_callback_handler(
    pool: web::Data<PgPool>,
    state_data: web::Data<OAuthAuthState>,
    query: web::Query<OAuthCallbackQuery>,
) -> HttpResponse {
    // 检查是否有错误
    if let Some(ref error) = query.error {
        return HttpResponse::BadRequest().json(OAuthErrorResponse {
            error: error.clone(),
            message: query.error_description.clone(),
        });
    }

    // 验证 state 参数
    let state_info = match validate_oauth_state(&pool, &query.state).await {
        Ok(Some(info)) => info,
        Ok(None) => {
            return HttpResponse::BadRequest().json(OAuthErrorResponse {
                error: "invalid_state".to_string(),
                message: Some("Invalid or expired state parameter".to_string()),
            });
        }
        Err(e) => {
            log::error!("Failed to validate OAuth state: {}", e);
            return HttpResponse::InternalServerError().json(OAuthErrorResponse {
                error: "internal_error".to_string(),
                message: Some("Failed to validate state".to_string()),
            });
        }
    };

    // 获取身份提供商配置
    let provider = match get_provider_by_id(&pool, &state_info.provider_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return HttpResponse::BadRequest().json(OAuthErrorResponse {
                error: "provider_not_found".to_string(),
                message: Some("Identity provider not found".to_string()),
            });
        }
        Err(e) => {
            log::error!("Failed to get provider: {}", e);
            return HttpResponse::InternalServerError().json(OAuthErrorResponse {
                error: "internal_error".to_string(),
                message: Some("Failed to get provider configuration".to_string()),
            });
        }
    };

    // 交换授权码获取令牌（平台分发）
    let token_response =
        match exchange_code_for_provider(&provider, &query.code, &state_info.pkce_verifier).await {
            Ok(token) => token,
            Err(e) => {
                log::error!("Failed to exchange code for {}: {}", provider.name, e);
                return HttpResponse::BadRequest().json(OAuthErrorResponse {
                    error: "token_exchange_failed".to_string(),
                    message: Some(format!("Failed to exchange authorization code: {}", e)),
                });
            }
        };

    // 获取用户信息（平台分发）
    let user_info = match get_user_info_for_provider(&provider, &token_response).await {
        Ok(info) => info,
        Err(e) => {
            log::error!("Failed to get user info from {}: {}", provider.name, e);
            return HttpResponse::BadRequest().json(OAuthErrorResponse {
                error: "user_info_failed".to_string(),
                message: Some(format!("Failed to get user information: {}", e)),
            });
        }
    };

    // 查找或创建用户
    let user = match find_or_create_user(&pool, &provider, &user_info, &token_response).await {
        Ok(user) => user,
        Err(e) => {
            log::error!("Failed to find or create user: {}", e);
            return HttpResponse::InternalServerError().json(OAuthErrorResponse {
                error: "user_creation_failed".to_string(),
                message: Some("Failed to create or update user".to_string()),
            });
        }
    };

    // 生成 JWT
    let claims = Claims::with_expiry_seconds(
        &user.id,
        user.email.as_deref().unwrap_or(""),
        false, // OAuth 登录不视为 MFA 验证
        state_data.jwt_access_expiry_secs,
    );

    let access_token = match encode_access_token(&claims, &state_data.jwt_private_key) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(OAuthErrorResponse {
                error: "token_generation_failed".to_string(),
                message: Some("Failed to generate access token".to_string()),
            });
        }
    };

    let refresh_token = match encode_refresh_token(
        &claims,
        &state_data.jwt_private_key,
        state_data.jwt_refresh_expiry_secs,
    ) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(OAuthErrorResponse {
                error: "token_generation_failed".to_string(),
                message: Some("Failed to generate refresh token".to_string()),
            });
        }
    };

    // 构建重定向响应——浏览器跳到 redirect_url，Gateway LoginPage 通过 ?token= 拾取
    let redirect_to = format!(
        "{}?token={}",
        state_info.redirect_url.trim_end_matches('/'),
        urlencoding::encode(&access_token),
    );
    let response = HttpResponse::Found()
        .append_header(("Location", redirect_to.as_str()))
        .finish();

    set_refresh_cookie(response, &refresh_token, state_data.jwt_refresh_expiry_secs)
}

/// 获取身份提供商配置
async fn get_provider_config(
    pool: &PgPool,
    name: &str,
) -> Result<Option<(ProviderConfig, bool)>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT id, name, provider_type, authorization_endpoint, token_endpoint,
               userinfo_endpoint, jwks_uri, client_id, client_secret_encrypted,
               scopes, field_mapping, enabled
        FROM isahl_auth.identity_providers
        WHERE name = $1
        "#,
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => {
            let enabled: bool = row.get("enabled");
            Ok(Some((
                ProviderConfig {
                    id: row.get("id"),
                    name: row.get("name"),
                    provider_type: row.get("provider_type"),
                    authorization_endpoint: row.get("authorization_endpoint"),
                    token_endpoint: row.get("token_endpoint"),
                    userinfo_endpoint: row.get("userinfo_endpoint"),
                    _jwks_uri: row.get("jwks_uri"),
                    client_id: row.get("client_id"),
                    client_secret: row.get("client_secret_encrypted"),
                    scopes: row.get::<Vec<String>, _>("scopes").join(" "),
                    field_mapping: row.get("field_mapping"),
                },
                enabled,
            )))
        }
        None => Ok(None),
    }
}

/// 根据 ID 获取身份提供商
async fn get_provider_by_id(
    pool: &PgPool,
    id: &i64,
) -> Result<Option<ProviderConfig>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT id, name, provider_type, authorization_endpoint, token_endpoint,
               userinfo_endpoint, jwks_uri, client_id, client_secret_encrypted,
               scopes, field_mapping, enabled
        FROM isahl_auth.identity_providers
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => Ok(Some(ProviderConfig {
            id: row.get("id"),
            name: row.get("name"),
            provider_type: row.get("provider_type"),
            authorization_endpoint: row.get("authorization_endpoint"),
            token_endpoint: row.get("token_endpoint"),
            userinfo_endpoint: row.get("userinfo_endpoint"),
            _jwks_uri: row.get("jwks_uri"),
            client_id: row.get("client_id"),
            client_secret: row.get("client_secret_encrypted"),
            scopes: row.get::<Vec<String>, _>("scopes").join(" "),
            field_mapping: row.get("field_mapping"),
        })),
        None => Ok(None),
    }
}

/// 存储 OAuth state
async fn store_oauth_state(
    pool: &PgPool,
    state: &str,
    pkce_verifier: &PkceVerifier,
    provider_id: &i64,
    user_id: Option<&str>,
    redirect_url: &str,
) -> Result<(), sqlx::Error> {
    // 计算 PKCE verifier 的 SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(pkce_verifier.as_str());
    let verifier_hash = hex::encode(hasher.finalize());

    sqlx::query(
        r#"
        INSERT INTO isahl_auth.oauth_states (state, pkce_code_verifier_hash, provider_id, user_id, redirect_url, expires_at)
        VALUES ($1, $2, $3, $4, $5, NOW() + INTERVAL '10 minutes')
        "#
    )
    .bind(state)
    .bind(verifier_hash)
    .bind(provider_id)
    .bind(user_id)
    .bind(redirect_url)
    .execute(pool)
    .await?;

    Ok(())
}

/// 验证 OAuth state
async fn validate_oauth_state(
    pool: &PgPool,
    state: &str,
) -> Result<Option<OAuthStateInfo>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT id, provider_id, pkce_code_verifier_hash, user_id, redirect_url
        FROM isahl_auth.oauth_states
        WHERE state = $1
          AND used = false
          AND expires_at > NOW()
        FOR UPDATE
        "#,
    )
    .bind(state)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => {
            let id: i64 = row.get("id");
            let provider_id: i64 = row.get("provider_id");
            let pkce_verifier: String = row.get("pkce_code_verifier_hash");
            let user_id: Option<String> = row.get("user_id");
            let redirect_url: String = row.get("redirect_url");

            // 标记为已使用
            sqlx::query(
                r#"
                UPDATE isahl_auth.oauth_states
                SET used = true, used_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(id)
            .execute(pool)
            .await?;

            Ok(Some(OAuthStateInfo {
                id,
                provider_id,
                pkce_verifier,
                user_id,
                redirect_url,
            }))
        }
        None => Ok(None),
    }
}

/// 获取用户信息
async fn get_user_info(
    provider: &ProviderConfig,
    token_response: &TokenResponse,
) -> Result<NormalizedUserInfo, Box<dyn std::error::Error>> {
    // 如果是 OIDC 且有 ID Token，优先从 ID Token 获取
    if provider.provider_type == "oidc" {
        if let Some(ref _id_token) = token_response.id_token {
            // ID Token 验证需要完整的 OIDC 客户端实现
            // 这里简化处理，直接使用 UserInfo 端点
            log::info!("OIDC provider detected, using UserInfo endpoint");
        }
    }

    // 从 UserInfo 端点获取
    if let Some(ref userinfo_endpoint) = provider.userinfo_endpoint {
        let client = crate::http_client::get().clone();
        let response = client
            .get(userinfo_endpoint)
            .header(
                "Authorization",
                format!("Bearer {}", token_response.access_token),
            )
            .send()
            .await?;

        if response.status().is_success() {
            let data: serde_json::Value = response.json().await?;
            return Ok(extract_user_info_from_userinfo(
                &data,
                &provider.field_mapping,
            ));
        }
    }

    Err("Failed to get user info".into())
}

/// 查找或创建用户
async fn find_or_create_user(
    pool: &PgPool,
    provider: &ProviderConfig,
    user_info: &NormalizedUserInfo,
    token_response: &TokenResponse,
) -> Result<UserInfo, sqlx::Error> {
    // 首先尝试查找已绑定的 OAuth 账户
    let existing = sqlx::query_as::<_, (i64, Option<String>, bool)>(
        r#"
        SELECT u.id, u.email, u.mfa_enabled
        FROM isahl_auth.auth_users u
        JOIN isahl_auth.user_oauth_accounts oa ON u.id = oa.user_id
        WHERE oa.provider_id = $1
          AND oa.provider_user_id = $2
        "#,
    )
    .bind(provider.id)
    .bind(&user_info.id)
    .fetch_optional(pool)
    .await?;

    if let Some((id, email, mfa_enabled)) = existing {
        // 更新令牌信息
        sqlx::query(
            r#"
            UPDATE isahl_auth.user_oauth_accounts
            SET access_token_encrypted = $1,
                refresh_token_encrypted = $2,
                token_expires_at = $3,
                updated_at = NOW()
            WHERE provider_id = $4 AND provider_user_id = $5
            "#,
        )
        .bind(&token_response.access_token)
        .bind(token_response.refresh_token.as_ref())
        .bind(
            token_response
                .expires_in
                .map(|secs| Utc::now() + chrono::Duration::seconds(secs)),
        )
        .bind(provider.id)
        .bind(&user_info.id)
        .execute(pool)
        .await?;

        return Ok(UserInfo {
            id: id.to_string(),
            email,
            _mfa_enabled: mfa_enabled,
        });
    }

    // 尝试通过邮箱查找现有用户
    if let Some(ref email) = user_info.email {
        let existing_user = sqlx::query_as::<_, (i64, Option<String>, bool)>(
            r#"
            SELECT id, email, COALESCE(mfa_enabled, false)
            FROM isahl_auth.auth_users
            WHERE email = $1
            "#,
        )
        .bind(email)
        .fetch_optional(pool)
        .await?;

        if let Some((id, user_email, mfa_enabled)) = existing_user {
            // 绑定 OAuth 账户到现有用户
            bind_oauth_account(pool, id, provider, user_info, token_response).await?;
            return Ok(UserInfo {
                id: id.to_string(),
                email: user_email,
                _mfa_enabled: mfa_enabled,
            });
        }
    }

    // 创建新用户 (使用 isahl_auth.auth_users，BIGINT 主键)
    // 注意：需要先创建用户获取 ID，然后绑定 OAuth 账户
    let email = user_info.email.clone();
    let display_name = user_info.name.clone();

    // 创建用户 (没有密码，只能通过 OAuth 登录)
    // ID 由 gen_next_zuid() 自动生成
    let user_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO isahl_auth.auth_users (name, email, password_hash, mfa_enabled, display_name, created_at, updated_at)
        VALUES ($1, $2, NULL, false, $3, NOW(), NOW())
        RETURNING id
        "#
    )
    .bind(user_info.name.as_deref().unwrap_or("oauth_users"))
    .bind(&email)
    .bind(&display_name)
    .fetch_one(pool)
    .await?;

    // 绑定 OAuth 账户
    bind_oauth_account(pool, user_id, provider, user_info, token_response).await?;

    Ok(UserInfo {
        id: user_id.to_string(),
        email,
        _mfa_enabled: false,
    })
}

/// 绑定 OAuth 账户到用户
async fn bind_oauth_account(
    pool: &PgPool,
    user_id: i64,
    provider: &ProviderConfig,
    user_info: &NormalizedUserInfo,
    token_response: &TokenResponse,
) -> Result<(), sqlx::Error> {
    // 检查是否已存在 OAuth 账户绑定
    let existing_count = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM isahl_auth.user_oauth_accounts WHERE user_id = $1"#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO isahl_auth.user_oauth_accounts (
            user_id, provider_id, provider_user_id, email, display_name,
            avatar_url, raw_profile, access_token_encrypted, refresh_token_encrypted,
            token_expires_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW(), NOW())
        ON CONFLICT (provider_id, provider_user_id) DO UPDATE
        SET email = EXCLUDED.email,
            display_name = EXCLUDED.display_name,
            avatar_url = EXCLUDED.avatar_url,
            raw_profile = EXCLUDED.raw_profile,
            access_token_encrypted = EXCLUDED.access_token_encrypted,
            refresh_token_encrypted = EXCLUDED.refresh_token_encrypted,
            token_expires_at = EXCLUDED.token_expires_at,
            updated_at = NOW()
        "#,
    )
    .bind(user_id)
    .bind(provider.id)
    .bind(&user_info.id)
    .bind(&user_info.email)
    .bind(&user_info.name)
    .bind(&user_info.picture)
    .bind(&user_info.raw)
    .bind(&token_response.access_token)
    .bind(token_response.refresh_token.as_ref())
    .bind(
        token_response
            .expires_in
            .map(|secs| Utc::now() + chrono::Duration::seconds(secs)),
    )
    .execute(pool)
    .await?;

    // 如果是账户关联（已存在其他 OAuth 账户），撤销旧的 refresh tokens
    if existing_count > 0 {
        let result = sqlx::query(
            r#"
            UPDATE isahl_auth.refresh_token_blocklist 
            SET revoked = true, revoked_at = NOW() 
            WHERE user_id = $1 AND revoked = false
            "#,
        )
        .bind(user_id)
        .execute(pool)
        .await?;

        let revoked_count = result.rows_affected();
        log::info!(
            "Account link token migration: user_id={}, provider={}, tokens_revoked={}",
            user_id,
            provider.name,
            revoked_count
        );
    }

    Ok(())
}

/// 获取基础 URL（指向 Gateway 前端，用于 OAuth 回调等场景）
///
/// 在开发环境中默认指向 Gateway 前端 Vite dev server (localhost:13000)。
/// 在生产环境中应设置 `APP_BASE_URL` 环境变量指向实际 Gateway 前端 URL。
/// 前端 URL 必须能通过 `GET /auth/oauth/callback` 路径访问 SSO callback 页面。
fn get_base_url() -> String {
    std::env::var("APPCREATOR_FRONTEND_URL")
        .ok()
        .or_else(|| std::env::var("APP_BASE_URL").ok())
        .or_else(|| std::env::var("GATEWAY_FRONTEND_URL").ok())
        .unwrap_or_else(|| "http://localhost:13000".to_string())
}

/// 应用状态
#[derive(Clone)]
pub struct OAuthAuthState {
    pub jwt_private_key: Vec<u8>,
    pub jwt_public_key: Vec<u8>,
    /// Access Token TTL（秒）
    pub jwt_access_expiry_secs: i64,
    /// Refresh Token TTL（秒）
    pub jwt_refresh_expiry_secs: i64,
}

/// 配置 OAuth 路由
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/oauth")
            .route("/login", web::post().to(oauth_login))
            .route("/callback", web::get().to(oauth_callback)),
    );
}

// ============================================================================
// 平台特定的 Token 交换
// ============================================================================

async fn exchange_code_for_provider(
    provider: &ProviderConfig,
    code: &str,
    pkce_verifier: &str,
) -> Result<TokenResponse, Box<dyn std::error::Error>> {
    match provider.name.as_str() {
        "WeChat" => exchange_wechat_code(provider, code).await,
        "WeCom" => exchange_wecom_code(provider, code).await,
        "Feishu" => exchange_feishu_code(provider, code).await,
        "DingTalk" => exchange_dingtalk_code(provider, code).await,
        _ => {
            // 标准 OAuth2
            let oauth_client = OAuth2Client::new(
                &provider.client_id,
                &provider.client_secret,
                &provider.authorization_endpoint,
                &provider.token_endpoint,
                format!("{}/auth/oauth/callback", get_base_url()),
            );
            oauth_client
                .exchange_code(code, Some(pkce_verifier))
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
        }
    }
}

async fn exchange_wechat_code(
    provider: &ProviderConfig,
    code: &str,
) -> Result<TokenResponse, Box<dyn std::error::Error>> {
    let url = format!(
        "{}?appid={}&secret={}&code={}&grant_type=authorization_code",
        provider.token_endpoint,
        urlencoding::encode(&provider.client_id),
        urlencoding::encode(&provider.client_secret),
        urlencoding::encode(code)
    );

    let client = crate::http_client::get().clone();
    let resp = client.get(&url).send().await?;
    let data: serde_json::Value = resp.json().await?;

    if data.get("errcode").is_some() {
        return Err(format!(
            "WeChat error: {} - {}",
            data.get("errcode").and_then(|v| v.as_i64()).unwrap_or(0),
            data.get("errmsg").and_then(|v| v.as_str()).unwrap_or("")
        )
        .into());
    }

    Ok(TokenResponse {
        access_token: data["access_token"].as_str().unwrap_or("").to_string(),
        token_type: data["token_type"].as_str().unwrap_or("Bearer").to_string(),
        expires_in: data["expires_in"].as_i64(),
        refresh_token: data["refresh_token"].as_str().map(|s| s.to_string()),
        scope: data["scope"].as_str().map(|s| s.to_string()),
        id_token: data["openid"].as_str().map(|s| s.to_string()), // reuse id_token field for openid
    })
}

async fn exchange_feishu_code(
    provider: &ProviderConfig,
    code: &str,
) -> Result<TokenResponse, Box<dyn std::error::Error>> {
    let client = crate::http_client::get().clone();
    let resp = client
        .post(&provider.token_endpoint)
        .header("Content-Type", "application/json; charset=utf-8")
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "code": code
        }))
        .send()
        .await?;

    let data: serde_json::Value = resp.json().await?;

    if data["code"].as_i64() != Some(0) {
        return Err(format!(
            "Feishu error: {} - {}",
            data["code"].as_i64().unwrap_or(0),
            data["msg"].as_str().unwrap_or("")
        )
        .into());
    }

    let token_data = &data["data"];
    Ok(TokenResponse {
        access_token: token_data["access_token"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        token_type: "Bearer".to_string(),
        expires_in: token_data["expires_in"].as_i64(),
        refresh_token: token_data["refresh_token"].as_str().map(|s| s.to_string()),
        scope: None,
        id_token: None,
    })
}

async fn exchange_dingtalk_code(
    provider: &ProviderConfig,
    code: &str,
) -> Result<TokenResponse, Box<dyn std::error::Error>> {
    let client = crate::http_client::get().clone();
    let resp = client
        .post(&provider.token_endpoint)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "clientId": provider.client_id,
            "clientSecret": provider.client_secret,
            "code": code,
            "grantType": "authorization_code"
        }))
        .send()
        .await?;

    let data: serde_json::Value = resp.json().await?;

    if data["success"].as_bool() != Some(true) {
        return Err(format!(
            "DingTalk error: {}",
            data["errMsg"].as_str().unwrap_or("Unknown error")
        )
        .into());
    }

    let token_data = &data["data"];
    Ok(TokenResponse {
        access_token: token_data["accessToken"].as_str().unwrap_or("").to_string(),
        token_type: "Bearer".to_string(),
        expires_in: token_data["expireIn"].as_i64(),
        refresh_token: token_data["refreshToken"].as_str().map(|s| s.to_string()),
        scope: None,
        id_token: None,
    })
}

// ============================================================================
// 平台特定的用户信息获取
// ============================================================================

async fn get_user_info_for_provider(
    provider: &ProviderConfig,
    token_response: &TokenResponse,
) -> Result<NormalizedUserInfo, Box<dyn std::error::Error>> {
    match provider.name.as_str() {
        "WeChat" => get_wechat_user_info(provider, token_response).await,
        "WeCom" => get_wecom_user_info(provider, token_response).await,
        "Feishu" => get_feishu_user_info(provider, token_response).await,
        "DingTalk" => get_dingtalk_user_info(provider, token_response).await,
        _ => get_user_info(provider, token_response).await,
    }
}

async fn get_wechat_user_info(
    provider: &ProviderConfig,
    token_response: &TokenResponse,
) -> Result<NormalizedUserInfo, Box<dyn std::error::Error>> {
    let openid = token_response.id_token.as_deref().unwrap_or("");
    let url = format!(
        "{}?access_token={}&openid={}",
        provider
            .userinfo_endpoint
            .as_deref()
            .unwrap_or("https://api.weixin.qq.com/sns/userinfo"),
        urlencoding::encode(&token_response.access_token),
        urlencoding::encode(openid)
    );

    let client = crate::http_client::get().clone();
    let resp = client.get(&url).send().await?;
    let data: serde_json::Value = resp.json().await?;

    if data.get("errcode").is_some() {
        return Err(format!(
            "WeChat userinfo error: {} - {}",
            data["errcode"].as_i64().unwrap_or(0),
            data["errmsg"].as_str().unwrap_or("")
        )
        .into());
    }

    Ok(NormalizedUserInfo {
        id: data["openid"].as_str().unwrap_or("").to_string(),
        email: None,
        name: data["nickname"].as_str().map(|s| s.to_string()),
        picture: data["headimgurl"].as_str().map(|s| s.to_string()),
        raw: data,
    })
}

async fn get_feishu_user_info(
    provider: &ProviderConfig,
    token_response: &TokenResponse,
) -> Result<NormalizedUserInfo, Box<dyn std::error::Error>> {
    let client = crate::http_client::get().clone();
    let resp = client
        .get(
            provider
                .userinfo_endpoint
                .as_deref()
                .unwrap_or("https://open.feishu.cn/open-apis/authen/v1/user_info"),
        )
        .header(
            "Authorization",
            format!("Bearer {}", token_response.access_token),
        )
        .send()
        .await?;

    let data: serde_json::Value = resp.json().await?;

    if data["code"].as_i64() != Some(0) {
        return Err(format!(
            "Feishu userinfo error: {} - {}",
            data["code"].as_i64().unwrap_or(0),
            data["msg"].as_str().unwrap_or("")
        )
        .into());
    }

    let user_data = &data["data"];
    Ok(NormalizedUserInfo {
        id: user_data["open_id"].as_str().unwrap_or("").to_string(),
        email: user_data["email"].as_str().map(|s| s.to_string()),
        name: user_data["name"].as_str().map(|s| s.to_string()),
        picture: user_data["avatar_url"].as_str().map(|s| s.to_string()),
        raw: data.clone(),
    })
}

async fn get_dingtalk_user_info(
    provider: &ProviderConfig,
    token_response: &TokenResponse,
) -> Result<NormalizedUserInfo, Box<dyn std::error::Error>> {
    let client = crate::http_client::get().clone();
    let resp = client
        .get(
            provider
                .userinfo_endpoint
                .as_deref()
                .unwrap_or("https://api.dingtalk.com/v1.0/contact/users/me"),
        )
        .header("x-acs-dingtalk-access-token", &token_response.access_token)
        .send()
        .await?;

    let data: serde_json::Value = resp.json().await?;

    if !data["nick"].is_string() && data["errMsg"].is_string() {
        return Err(format!(
            "DingTalk userinfo error: {}",
            data["errMsg"].as_str().unwrap_or("Unknown error")
        )
        .into());
    }

    Ok(NormalizedUserInfo {
        id: data["openId"].as_str().unwrap_or("").to_string(),
        email: data["email"].as_str().map(|s| s.to_string()),
        name: data["nick"].as_str().map(|s| s.to_string()),
        picture: data["avatarUrl"].as_str().map(|s| s.to_string()),
        raw: data,
    })
}

async fn exchange_wecom_code(
    provider: &ProviderConfig,
    code: &str,
) -> Result<TokenResponse, Box<dyn std::error::Error>> {
    // Step 1: 获取企业微信应用 access_token
    let token_url = format!(
        "{}?corpid={}&corpsecret={}",
        provider.token_endpoint,
        urlencoding::encode(&provider.client_id),
        urlencoding::encode(&provider.client_secret)
    );

    let client = crate::http_client::get().clone();
    let resp = client.get(&token_url).send().await?;
    let token_data: serde_json::Value = resp.json().await?;

    if token_data["errcode"].as_i64() != Some(0) {
        return Err(format!(
            "WeCom gettoken error: {} - {}",
            token_data["errcode"].as_i64().unwrap_or(0),
            token_data["errmsg"].as_str().unwrap_or("")
        )
        .into());
    }

    let access_token = token_data["access_token"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // Step 2: 用 code 获取 userid
    let userinfo_url = format!(
        "https://qyapi.weixin.qq.com/cgi-bin/user/getuserinfo?access_token={}&code={}",
        urlencoding::encode(&access_token),
        urlencoding::encode(code)
    );

    let resp = client.get(&userinfo_url).send().await?;
    let user_data: serde_json::Value = resp.json().await?;

    if user_data["errcode"].as_i64() != Some(0) {
        return Err(format!(
            "WeCom getuserinfo error: {} - {}",
            user_data["errcode"].as_i64().unwrap_or(0),
            user_data["errmsg"].as_str().unwrap_or("")
        )
        .into());
    }

    let userid = user_data["UserId"]
        .as_str()
        .or_else(|| user_data["userid"].as_str())
        .unwrap_or("")
        .to_string();

    Ok(TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: token_data["expires_in"].as_i64(),
        refresh_token: None,
        scope: None,
        id_token: Some(userid), // reuse id_token for userid
    })
}

async fn get_wecom_user_info(
    provider: &ProviderConfig,
    token_response: &TokenResponse,
) -> Result<NormalizedUserInfo, Box<dyn std::error::Error>> {
    let userid = token_response.id_token.as_deref().unwrap_or("");
    let url = format!(
        "{}?access_token={}&userid={}",
        provider
            .userinfo_endpoint
            .as_deref()
            .unwrap_or("https://qyapi.weixin.qq.com/cgi-bin/user/get"),
        urlencoding::encode(&token_response.access_token),
        urlencoding::encode(userid)
    );

    let client = crate::http_client::get().clone();
    let resp = client.get(&url).send().await?;
    let data: serde_json::Value = resp.json().await?;

    if data["errcode"].as_i64() != Some(0) {
        return Err(format!(
            "WeCom user get error: {} - {}",
            data["errcode"].as_i64().unwrap_or(0),
            data["errmsg"].as_str().unwrap_or("")
        )
        .into());
    }

    Ok(NormalizedUserInfo {
        id: data["userid"].as_str().unwrap_or("").to_string(),
        email: data["email"].as_str().map(|s| s.to_string()),
        name: data["name"].as_str().map(|s| s.to_string()),
        picture: data["avatar"].as_str().map(|s| s.to_string()),
        raw: data,
    })
}
