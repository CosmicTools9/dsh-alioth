//! 微信小程序一键登录（wechat-miniapp-one-tap-login）。
//!
//! 设计裁决（2026-09-23 用户指令）：
//! - **每次登录都重新取手机号**（仅 `requires_phone` 来源）——不做 openid 静默登录通道，
//!   不回写 openid/unionid 至 `user_oauth_accounts`（零 DDL、零数据迁移）；两者仅**转交**
//!   来源 provider 用于其自身身份锚定（C 端以 unionid 优先归并，见 change
//!   `add-cross-appid-user-merging`）；
//! - **司机端不注册**：手机号未命中司机 → `DRIVER_NOT_FOUND`；命中但自然人 `fk_user`
//!   未绑登录账号 → `DRIVER_UNBOUND`（账号须由承运商登记司机时经 `user` 字段预绑，
//!   见 OpenActivity `portal_write::create_driver`）；
//! - `fromType` 标识小程序来源；来源的**身份锚点**由
//!   [`common::wechat_miniapp::WechatMiniappLogin`] provider 决定
//!   （司机端=手机号，C 端=openid 即账户）。
//!
//! 流程：
//!   ① fromType → 注册表查 provider（未注册/凭据缺失 → 400 INVALID_FROM_TYPE）；
//!   ② js_code → `sns/jscode2session`（校验微信侧登录态）；
//!   ③ `requires_phone` 时 phone_code → `wxa/business/getuserphonenumber`
//!      （全局 access_token 经 `cgi-bin/token` 换取，进程内缓存 + 过期前 5 分钟刷新）
//!      → 明文手机号；
//!   ④ provider 锚定身份（来源各自语义：司机查司机表 / C 端 openid upsert 自助注册）；
//!   ⑤ 账号状态门禁 `isahl_auth.auth_users`（is_active + 状态门禁与密码登录同口径，
//!      MFA fail-closed）；
//!   ⑥ 签发后段复用 `login::handlers::issue_login_response`（sso_sessions 建会话 +
//!      ES256 JWT + refresh 轮换 + scope cookie），响应与 `/auth/login` 同构。
//!
//! **扩展性**（bv-local-miniapp-sso-wechat-login-extension）：第 ④ 步下沉 provider 后，
//! 新增一个小程序 = 实现 trait + 组合根注册一行，本文件不再改。

use actix_web::{web, HttpRequest, HttpResponse};
use common::wechat_miniapp::{WechatLoginContext, WechatLoginError};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::login::AuthError;
use super::AuthState;

/// 一键登录请求体（fromType = 小程序来源标识；js_code = wx.login() 凭证；
/// phone_code = getPhoneNumber 回调的一次性手机号凭证——**仅 `requires_phone`
/// 来源必填**，C 端（openid 即账户）不传）。
#[derive(Debug, serde::Deserialize)]
pub struct WxMiniappLoginRequest {
    #[serde(rename = "fromType")]
    pub from_type: String,
    pub js_code: String,
    #[serde(default)]
    pub phone_code: String,
}

/// 微信 API 错误摘要（errcode/errmsg），用于日志。
fn wx_err(tag: &str, data: &serde_json::Value) -> String {
    format!(
        "{tag}: errcode={} errmsg={}",
        data.get("errcode").and_then(|v| v.as_i64()).unwrap_or(0),
        data.get("errmsg").and_then(|v| v.as_str()).unwrap_or("")
    )
}

/// js_code → openid/session_key（sns/jscode2session）。openid 仅用于审计日志。
async fn code2session(
    appid: &str,
    secret: &str,
    js_code: &str,
) -> Result<serde_json::Value, String> {
    let url = format!(
        "https://api.weixin.qq.com/sns/jscode2session?appid={}&secret={}&js_code={}&grant_type=authorization_code",
        urlencoding::encode(appid),
        urlencoding::encode(secret),
        urlencoding::encode(js_code)
    );
    let client = crate::http_client::get().clone();
    let data: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("jscode2session 请求失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("jscode2session 响应解析失败: {e}"))?;
    let errcode = data.get("errcode").and_then(|v| v.as_i64()).unwrap_or(0);
    if errcode != 0 {
        return Err(wx_err("jscode2session", &data));
    }
    if data.get("openid").and_then(|v| v.as_str()).is_none() {
        return Err("jscode2session: 响应缺少 openid".to_string());
    }
    Ok(data)
}

/// 全局 access_token 缓存项（7200s 有效期，过期前 5 分钟即视为失效强制刷新）。
struct CachedAccessToken {
    token: String,
    expires_at: Instant,
}

/// 进程级缓存（key = appid，预留多小程序并存）。标准库 Mutex——临界区不含 await。
static ACCESS_TOKEN_CACHE: LazyLock<Mutex<HashMap<String, CachedAccessToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 取小程序全局 access_token（cgi-bin/token?grant_type=client_credential）。
async fn wechat_access_token(appid: &str, secret: &str) -> Result<String, String> {
    let cache = &*ACCESS_TOKEN_CACHE;
    {
        let guard = cache.lock().expect("access_token 缓存锁中毒");
        if let Some(c) = guard.get(appid) {
            if Instant::now() < c.expires_at {
                return Ok(c.token.clone());
            }
        }
    }
    let url = format!(
        "https://api.weixin.qq.com/cgi-bin/token?grant_type=client_credential&appid={}&secret={}",
        urlencoding::encode(appid),
        urlencoding::encode(secret)
    );
    let client = crate::http_client::get().clone();
    let data: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("cgi-bin/token 请求失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("cgi-bin/token 响应解析失败: {e}"))?;
    let token = data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| wx_err("cgi-bin/token", &data))?
        .to_string();
    let expires_in = data
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(7200);
    let mut guard = cache.lock().expect("access_token 缓存锁中毒");
    guard.insert(
        appid.to_string(),
        CachedAccessToken {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs((expires_in - 300).max(60) as u64),
        },
    );
    Ok(token)
}

/// phone_code → 明文手机号（wxa/business/getuserphonenumber，返回数字归一后的号码——
/// 与门户司机判重 `norm_phone_digits` 同口径：只保留数字）。
async fn fetch_phone_number(access_token: &str, phone_code: &str) -> Result<String, String> {
    let url = format!(
        "https://api.weixin.qq.com/wxa/business/getuserphonenumber?access_token={}",
        urlencoding::encode(access_token)
    );
    let client = crate::http_client::get().clone();
    let data: serde_json::Value = client
        .post(&url)
        .json(&serde_json::json!({ "code": phone_code }))
        .send()
        .await
        .map_err(|e| format!("getuserphonenumber 请求失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("getuserphonenumber 响应解析失败: {e}"))?;
    let errcode = data.get("errcode").and_then(|v| v.as_i64()).unwrap_or(-1);
    if errcode != 0 {
        return Err(wx_err("getuserphonenumber", &data));
    }
    let info = data
        .get("phone_info")
        .ok_or_else(|| "getuserphonenumber: 响应缺少 phone_info".to_string())?;
    // purePhoneNumber = 不带区号的号码（优先）；phoneNumber = 带区号完整号（回退）
    let raw = info
        .get("purePhoneNumber")
        .or_else(|| info.get("phoneNumber"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "getuserphonenumber: phone_info 缺少号码".to_string())?;
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err("getuserphonenumber: 号码归一后为空".to_string());
    }
    Ok(digits)
}

/// POST /auth/wechat/miniapp/login — 微信小程序一键登录（来源 provider 锚定身份 → fk_user → 签发登录态）
pub async fn miniapp_login(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    registry: web::Data<common::wechat_miniapp::WechatMiniappRegistry>,
    req: HttpRequest,
    body: web::Json<WxMiniappLoginRequest>,
) -> HttpResponse {
    let b = body.into_inner();
    if b.from_type.trim().is_empty() || b.js_code.trim().is_empty() {
        return HttpResponse::BadRequest().json(AuthError {
            error: "INVALID_REQUEST: fromType/js_code 必填".to_string(),
        });
    }
    let from_type = b.from_type.trim();

    // ① fromType → 注册表查 provider（未注册/凭据缺失 → 400，fail-closed）
    let Some(provider) = registry.resolve(from_type) else {
        log::warn!("小程序一键登录: 未注册的 fromType={from_type}");
        return HttpResponse::BadRequest().json(AuthError {
            error: "INVALID_FROM_TYPE: 登录通道未注册".to_string(),
        });
    };
    let Some((appid, secret)) = provider.credentials() else {
        log::warn!("小程序一键登录: fromType={from_type} 凭据未配置");
        return HttpResponse::BadRequest().json(AuthError {
            error: "INVALID_FROM_TYPE: 登录通道未配置".to_string(),
        });
    };
    // 该来源是否要求手机号授权（C 端 openid 锚点不需要，司机端手机号锚点需要）
    let requires_phone = provider.requires_phone();

    // ② js_code → openid（校验微信侧登录态）
    let session_data = match code2session(&appid, &secret, b.js_code.trim()).await {
        Ok(d) => d,
        Err(e) => {
            log::warn!("小程序一键登录 jscode2session 失败: {e}");
            return HttpResponse::BadRequest().json(AuthError {
                error: "WX_CODE_INVALID: 微信登录凭证无效或已过期".to_string(),
            });
        }
    };
    let openid = session_data["openid"].as_str().unwrap_or("").to_string();
    // unionid：同一微信开放平台账号（主体）下跨应用一致；该 AppID 未绑定时为空串。
    // 仅**转交**来源 provider（其自身身份锚定用），不在平台侧持久化、不推导。
    let unionid = session_data["unionid"].as_str().unwrap_or("").to_string();

    // ③ requires_phone 时才取手机号（C 端不弹授权框）
    let phone = if requires_phone {
        if b.phone_code.trim().is_empty() {
            return HttpResponse::BadRequest().json(AuthError {
                error: "INVALID_REQUEST: phone_code 必填".to_string(),
            });
        }
        let access_token = match wechat_access_token(&appid, &secret).await {
            Ok(t) => t,
            Err(e) => {
                log::error!("小程序一键登录取 access_token 失败: {e}");
                return HttpResponse::BadGateway().json(AuthError {
                    error: "WX_API_ERROR: 微信服务暂不可用，请稍后重试".to_string(),
                });
            }
        };
        match fetch_phone_number(&access_token, b.phone_code.trim()).await {
            Ok(p) => p,
            Err(e) => {
                log::warn!("小程序一键登录 getuserphonenumber 失败: {e}");
                return HttpResponse::BadRequest().json(AuthError {
                    error: "WX_CODE_INVALID: 手机号凭证无效或已过期，请重新授权".to_string(),
                });
            }
        }
    } else {
        String::new()
    };

    // ④ 身份锚定（来源各自语义：司机查司机表不注册 / C 端 openid upsert 自助注册）
    let ctx = WechatLoginContext {
        openid: openid.clone(),
        unionid,
        phone: phone.clone(),
        requires_phone,
    };
    let user_id = match provider.resolve_identity(pool.get_ref(), &ctx).await {
        Ok(u) => u,
        Err(WechatLoginError::NotFound) => {
            log::info!("小程序一键登录: {from_type} 未命中用户 openid={openid}");
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "DRIVER_NOT_FOUND",
                "message": "未找到司机，请联系调度人员处理",
            }));
        }
        Err(WechatLoginError::Unbound) => {
            log::warn!("小程序一键登录: {from_type} 命中但未绑定登录账号 openid={openid}");
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "DRIVER_UNBOUND",
                "message": "司机未绑定登录账号，请联系调度人员处理",
            }));
        }
        Err(WechatLoginError::Unavailable(e)) => {
            log::error!("小程序一键登录: {from_type} 身份锚定失败: {e}");
            return HttpResponse::InternalServerError().json(AuthError {
                error: "DB_ERROR".to_string(),
            });
        }
    };

    // ⑤ 账号状态门禁（所有来源同一口径：与密码登录一致）
    let account = sqlx::query_as::<_, (Option<String>, bool, bool)>(
        "SELECT status::text, is_active, COALESCE(mfa_enabled, false) \
         FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await;
    let (status, is_active, mfa_enabled) = match account {
        Ok(Some(a)) => a,
        Ok(None) => {
            log::warn!("小程序一键登录: {from_type} 指向的账号不存在 user={user_id}");
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "USER_INACTIVE",
                "message": "登录账号不可用，请联系调度人员处理",
            }));
        }
        Err(e) => {
            log::error!("小程序一键登录账号查询失败: {e}");
            return HttpResponse::InternalServerError().json(AuthError {
                error: "DB_ERROR".to_string(),
            });
        }
    };
    if !is_active {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "USER_INACTIVE",
            "message": "登录账号不可用，请联系调度人员处理",
        }));
    }
    // MFA 账号不支持一键登录（免密通道不得弱化二次验证——fail-closed）
    if mfa_enabled {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "MFA_NOT_SUPPORTED",
            "message": "该账号已开启二次验证，请使用账号密码登录",
        }));
    }
    // 状态门禁（与密码登录同一口径：active/rejected 放行）
    if let Some(resp) = super::login::status_gate_error(status.as_deref()) {
        return resp;
    }

    // ⑥ 签发登录态（与 /auth/login 同一实现：会话 + JWT + refresh + cookie）
    super::login::issue_login_response(pool.get_ref(), state.get_ref(), &req, user_id).await
}

#[cfg(test)]
mod miniapp_login_fail_closed_tests {
    //! 离线断言「来源归属归一各 ns」后的失败闭合面（change
    //! `extract-ns-business-providers-from-shared-sso` T4.2 的**可离线部分**）：
    //! · 平台侧不再持有任何来源（组合根传空集）⇒ 任意 fromType（含 NS 侧来源，如 WZ 的 `wz-driver-wechat`）未注册 ⇒ 400 `INVALID_FROM_TYPE`；
    //! · 来源已注册但凭据未配置（WZ 侧 provider 就位、凭据未配的形态）⇒ 同样 400。
    //! 两条路径都在 `jscode2session`（②）**之前**返回 ⇒ 不需 WeChat 网络、不需 DB（惰性 PgPool 不建连）。
    use super::{miniapp_login, WxMiniappLoginRequest};
    use crate::auth::AuthState;
    use actix_web::{web, HttpResponse};
    use async_trait::async_trait;
    use common::wechat_miniapp::{
        WechatLoginContext, WechatLoginError, WechatMiniappLogin, WechatMiniappRegistry,
    };
    use sqlx::PgPool;

    fn lazy_pool() -> web::Data<PgPool> {
        web::Data::new(PgPool::connect_lazy("postgres://localhost:5432/none").expect("lazy pool"))
    }

    fn state() -> web::Data<AuthState> {
        web::Data::new(AuthState {
            jwt_private_key: vec![1u8; 32],
            jwt_public_key: vec![1u8; 33],
            jwt_public_keys_prev: vec![],
            encryption_key: b"test-encryption-key-16bytes".to_vec(),
            ngac_preview_dir: None,
            jwt_access_expiry_secs: 900,
            jwt_refresh_expiry_secs: 604800,
            identity_verify_mode: "local".to_string(),
            identity_external_verify_url: None,
        })
    }

    /// 已注册但凭据缺失的 provider（= 某 ns provider 就位而凭据未配的形态）。
    struct UnconfiguredDriver;

    #[async_trait]
    impl WechatMiniappLogin for UnconfiguredDriver {
        fn login_source(&self) -> &str {
            "wz-driver-wechat"
        }
        fn credentials(&self) -> Option<(String, String)> {
            None
        }
        fn requires_phone(&self) -> bool {
            true
        }
        async fn resolve_identity(
            &self,
            _pool: &PgPool,
            _ctx: &WechatLoginContext,
        ) -> Result<i64, WechatLoginError> {
            unreachable!("凭据缺失的 provider 不得被调用到 resolve_identity")
        }
    }

    async fn status_and_body(
        from_type: &str,
        drivers: Vec<Box<dyn WechatMiniappLogin>>,
    ) -> (u16, String) {
        let req = actix_web::test::TestRequest::default().to_http_request();
        let body = web::Json(WxMiniappLoginRequest {
            from_type: from_type.to_string(),
            js_code: "js-code-offline".to_string(),
            phone_code: String::new(),
        });
        let resp: HttpResponse = miniapp_login(
            lazy_pool(),
            state(),
            web::Data::new(WechatMiniappRegistry::with_drivers(drivers)),
            req,
            body,
        )
        .await;
        let status = resp.status().as_u16();
        let bytes = actix_web::body::to_bytes(resp.into_body())
            .await
            .expect("response body");
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    /// 平台剥离后（组合根零来源）⇒ 未注册 ⇒ 400 `INVALID_FROM_TYPE`。
    #[actix_web::test]
    async fn stripped_platform_rejects_unregistered_driver() {
        let (status, body) = status_and_body("wz-driver-wechat", vec![]).await;
        assert_eq!(status, 400, "body={body}");
        assert!(
            body.contains("INVALID_FROM_TYPE: 登录通道未注册"),
            "body={body}"
        );
    }

    /// 来源已注册但凭据未配置 ⇒ 仍 400（fail-closed）。
    #[actix_web::test]
    async fn registered_but_unconfigured_source_is_fail_closed() {
        let (status, body) =
            status_and_body("wz-driver-wechat", vec![Box::new(UnconfiguredDriver)]).await;
        assert_eq!(status, 400, "body={body}");
        assert!(
            body.contains("INVALID_FROM_TYPE: 登录通道未配置"),
            "body={body}"
        );
    }

    /// 必填字段缺失 ⇒ 400 `INVALID_REQUEST`（前置门禁）。
    #[actix_web::test]
    async fn missing_fields_rejected() {
        let (status, body) = status_and_body("", vec![]).await;
        assert_eq!(status, 400, "body={body}");
        assert!(body.contains("INVALID_REQUEST"), "body={body}");
    }
}
