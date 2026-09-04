pub mod api_clients_common;
pub mod api_key;
pub mod check_access;
pub mod client_secret;
pub mod crypto;
pub mod email;
pub mod identity;
pub mod introspect;
pub mod jwt;
pub mod ldap;
pub mod login;
pub mod mfa;
pub mod mfa_management;
pub mod middleware;
pub mod notification_preferences;
pub mod oauth;
pub mod oauth_callback;
pub mod oidc;
pub mod oidc_provider;
pub mod password;
pub mod password_change;
pub mod portal;
pub mod profile;
pub mod register;
pub mod reset_password;
pub mod service_user;
pub mod session;
pub mod slo;
pub mod sms;
pub mod social;
pub mod token;
pub mod webauthn;
pub mod zchat;

/// 复用 login 流程的 refresh token DB 校验（zchat refresh grant 使用，
/// 不复制实现——见 zchat.rs handle_refresh_grant）。
pub(crate) use login::is_valid_refresh_token;

use actix_web::{HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

use self::jwt::{decode_token_any, Claims};

/// 从请求中解析当前 JWT 用户 id。
///
/// 与 `webauthn::extract_user_id` 语义一致（接收 `&AuthState`），作为 SSO 内提取
/// 当前用户的统一入口；调用方需自行处理返回 `Err(HttpResponse)` 的提前返回。
pub(crate) fn extract_user_id(req: &HttpRequest, state: &AuthState) -> Result<i64, HttpResponse> {
    let token = req
        .cookie("access_token")
        .map(|c| c.value().to_string())
        .or_else(|| {
            req.headers()
                .get(actix_web::http::header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|a| a.strip_prefix("Bearer "))
                .map(|t| t.to_string())
        });

    let token = match token {
        Some(t) => t,
        None => {
            return Err(HttpResponse::Unauthorized()
                .json(serde_json::json!({ "error": "No authentication token" })));
        }
    };

    let claims: Claims = match decode_token_any(&token, &state.verification_keys()) {
        Ok(c) => c,
        Err(_) => {
            return Err(HttpResponse::Unauthorized()
                .json(serde_json::json!({ "error": "Invalid or expired token" })));
        }
    };

    match claims.sub.parse::<i64>() {
        Ok(id) => Ok(id),
        Err(_) => Err(HttpResponse::Unauthorized()
            .json(serde_json::json!({ "error": "Invalid user ID in token" }))),
    }
}

fn default_access_expiry_secs() -> i64 {
    900 // 15 分钟
}
fn default_refresh_expiry_secs() -> i64 {
    604800 // 7 天
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthState {
    /// ES256 (EC P-256) 私钥（PEM 字节）
    pub jwt_private_key: Vec<u8>,
    /// 从私钥派生出的 ES256 公钥（PEM 字节），用于本地验证 token
    pub jwt_public_key: Vec<u8>,
    /// 轮换窗口内的历史 ES256 公钥（kid, PEM）——JWKS 双钥过渡，旧 token 窗口内可验
    #[serde(default)]
    pub jwt_public_keys_prev: Vec<(String, Vec<u8>)>,
    pub encryption_key: Vec<u8>,
    /// NGAC OA 页面预览截图目录（dev-only；未设 → preview 禁用）
    #[serde(default)]
    pub ngac_preview_dir: Option<String>,
    /// Access Token TTL（秒），来自 Config::jwt_access_expiry
    #[serde(default = "default_access_expiry_secs")]
    #[serde(with = "common::serde_zuid")]
    pub jwt_access_expiry_secs: i64,
    /// Refresh Token TTL（秒），来自 Config::jwt_refresh_expiry
    #[serde(default = "default_refresh_expiry_secs")]
    #[serde(with = "common::serde_zuid")]
    pub jwt_refresh_expiry_secs: i64,
    /// 身份证实模式（"local" | "external"），来自 Config::identity_verify_mode
    pub identity_verify_mode: String,
    /// 第三方身份证实 API URL，来自 Config::identity_external_verify_url
    pub identity_external_verify_url: Option<String>,
}

impl AuthState {
    /// 全部验签公钥（active 在前，历史 prev 在后），供多 key 验签遍历。
    /// 轮换窗口内旧 token（prev 钥签发）仍可验；prev 为空 = 单钥模式。
    pub fn verification_keys(&self) -> Vec<&[u8]> {
        let mut keys = Vec::with_capacity(1 + self.jwt_public_keys_prev.len());
        keys.push(self.jwt_public_key.as_slice());
        keys.extend(
            self.jwt_public_keys_prev
                .iter()
                .map(|(_, pem)| pem.as_slice()),
        );
        keys
    }

    /// 多 key 验签（active + prev 依次尝试），轮换窗口内旧 token 可验。
    pub fn decode_token(&self, token: &str) -> Result<Claims, crate::auth::jwt::JwtError> {
        crate::auth::jwt::decode_token_any(token, &self.verification_keys())
    }
}
