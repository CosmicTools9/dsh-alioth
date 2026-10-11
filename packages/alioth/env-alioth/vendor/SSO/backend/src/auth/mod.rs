pub mod api_clients_common;
pub mod api_key;
pub mod check_access;
pub mod client_secret;
pub mod crypto;
pub mod email;
pub mod identifier;
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
pub mod wechat_miniapp;
pub mod zchat;

/// 复用 login 流程的 refresh token DB 校验（zchat refresh grant 使用，
/// 不复制实现——见 zchat.rs handle_refresh_grant）。
pub(crate) use login::is_valid_refresh_token;

use actix_web::{HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

use self::jwt::{decode_token_any, Claims};

/// 公开前缀（`/auth/*`、`/api/auth/*`）下 **cookie 鉴权**端点的统一 CSRF 自检（design.md §4「自检面」）。
///
/// 这些端点在 `RequireAuth` 的 public 名单内（登录/注册/刷新/回调等），全局门不会对其执行 ⇒
/// **各 handler MUST 在自身鉴权成功之后**调用本函数：`Some(resp)` ⇒ 短路返回该 403。
///
/// 无凭据可解析（未登录 / 首次登录）⇒ `None`（**不得**因「浏览器带着旧 cookie 去登录」而误伤登录
/// 入口——登录 CSRF 不在本 change 语义面：请求尚无已认证会话可被冒用）。
/// 开关/安全方法/豁免面（refresh、logout）由 `csrf_gate` 内部判定。
pub(crate) fn csrf_self_check(req: &HttpRequest) -> Option<HttpResponse> {
    let (_, source) = common::csrf::resolve_credential(req)?;
    common::csrf::csrf_gate(req, source)
}

/// 从请求中解析当前 JWT 用户 id。
///
/// 与 `webauthn::extract_user_id` 语义一致（接收 `&AuthState`），作为 SSO 内提取
/// 当前用户的统一入口；调用方需自行处理返回 `Err(HttpResponse)` 的提前返回。
pub(crate) fn extract_user_id(req: &HttpRequest, state: &AuthState) -> Result<i64, HttpResponse> {
    let (id, _source) = extract_user_id_with_source(req, state)?;
    // CSRF 自检（design.md §4「自检面」）：本函数是**公开前缀**（`/auth/*`）下 cookie 鉴权端点的
    // 统一入口（webauthn / portal / social 等），这些端点不在 `RequireAuth` 受保护面内 ⇒ 门在此执行。
    // 无凭据（未登录）或安全方法、豁免面由 `csrf_gate` 内部判定 ⇒ 不误伤登录入口。
    if let Some(resp) = csrf_self_check(req) {
        return Err(resp);
    }
    Ok(id)
}

/// 提取访问令牌并给出**产出该令牌的提取分支**对应的凭据来源（CSRF 判据唯一输入，
/// design.md §3「实际完成鉴权的凭据来源」——MUST NOT 另立「存在某 cookie 即 Cookie」
/// 类启发式）。
///
/// **单一实现**（spec-audit F1 根因）：委托 `common::csrf::resolve_credential`，顺序 =
/// `auth` capability「访问令牌提取优先级」（scoped cookie → Bearer → 环境 cookie）。
/// 本仓此前的第二份顺序（cookie → Bearer、无 scope）已删除——它与 `RequireAuth` 走
/// `jwt::extract_token`（经 common，scope 化）本就冲突，同一应用两条鉴权链优先级不同。
pub(crate) fn extract_token_with_source(
    req: &HttpRequest,
) -> Option<(String, common::csrf::CredentialSource)> {
    common::csrf::resolve_credential(req)
}

/// [`extract_user_id`] 的带来源形态（CSRF 插入点用；与基函数同一提取/验签链路）。
pub(crate) fn extract_user_id_with_source(
    req: &HttpRequest,
    state: &AuthState,
) -> Result<(i64, common::csrf::CredentialSource), HttpResponse> {
    let (token, source) = match extract_token_with_source(req) {
        Some(v) => v,
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
        Ok(id) => Ok((id, source)),
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
