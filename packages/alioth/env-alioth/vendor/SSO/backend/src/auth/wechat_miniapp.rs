//! 微信小程序一键登录（wechat-miniapp-one-tap-login）。
//!
//! 设计裁决（2026-09-23 用户指令）：
//! - **每次登录都重新取手机号**——不做 openid 静默登录通道，不回写 openid/unionid
//!   （不落 `user_oauth_accounts`，零 DDL、零数据迁移）；
//! - **不注册**：手机号未命中司机 → `DRIVER_NOT_FOUND`；命中但自然人 `fk_user`
//!   未绑登录账号 → `DRIVER_UNBOUND`（账号须由承运商登记司机时经 `user` 字段预绑，
//!   见 OpenActivity `portal_write::create_driver`）；
//! - `fromType` 标识小程序来源（本期仅 `driverWeChatApp` 司机端；后续其他小程序
//!   各自注册 appid/secret 配置对）。
//!
//! 流程：
//!   ① fromType → 小程序 appid/secret（配置缺失/未知 → 400 INVALID_FROM_TYPE）；
//!   ② js_code → `sns/jscode2session`（校验微信侧登录态；openid 仅入日志，不落库）；
//!   ③ phone_code → `wxa/business/getuserphonenumber`（全局 access_token 经
//!      `cgi-bin/token` 换取，进程内缓存 + 过期前 5 分钟刷新）→ 明文手机号；
//!   ④ 手机号**数字归一**后查司机（`isahl."zc_id_empl-natural"` ⋈ 雇佣桥——与门户
//!      `driver_by_name_phone` 同口径，读径唯一实现，禁第二份同语义 SQL）；
//!   ⑤ 命中行 `fk_user` → `isahl_auth.auth_users`（is_active + 状态门禁与密码登录同口径）；
//!   ⑥ 签发后段复用 `login::handlers::issue_login_response`（sso_sessions 建会话 +
//!      ES256 JWT + refresh 轮换 + scope cookie），响应与 `/auth/login` 同构。

use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::login::AuthError;
use super::AuthState;
use crate::config::Config;

/// 一键登录请求体（fromType = 小程序来源标识；js_code = wx.login() 凭证；
/// phone_code = getPhoneNumber 回调的一次性手机号凭证——两者均单次有效）。
#[derive(Debug, serde::Deserialize)]
pub struct WxMiniappLoginRequest {
    #[serde(rename = "fromType")]
    pub from_type: String,
    pub js_code: String,
    pub phone_code: String,
}

/// 小程序配置解析（fromType → appid/secret）。预留多小程序：每种来源一对 env 配置；
/// 未知来源或未配置 → None（调用方返 400 INVALID_FROM_TYPE，fail-closed）。
fn miniapp_credentials(config: &Config, from_type: &str) -> Option<(String, String)> {
    match from_type {
        "driverWeChatApp" => Some((
            config.wechat_miniapp_driver_appid.clone()?,
            config.wechat_miniapp_driver_secret.clone()?,
        )),
        _ => None,
    }
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

/// POST /auth/wechat/miniapp/login — 微信小程序一键登录（手机号找司机 → fk_user → 签发登录态）
pub async fn miniapp_login(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    config: web::Data<Config>,
    req: HttpRequest,
    body: web::Json<WxMiniappLoginRequest>,
) -> HttpResponse {
    let b = body.into_inner();
    if b.from_type.trim().is_empty()
        || b.js_code.trim().is_empty()
        || b.phone_code.trim().is_empty()
    {
        return HttpResponse::BadRequest().json(AuthError {
            error: "INVALID_REQUEST: fromType/js_code/phone_code 均必填".to_string(),
        });
    }

    // ① fromType → appid/secret（fail-closed）
    let Some((appid, secret)) = miniapp_credentials(&config, b.from_type.trim()) else {
        log::warn!("小程序一键登录: 未知/未配置的 fromType={}", b.from_type);
        return HttpResponse::BadRequest().json(AuthError {
            error: "INVALID_FROM_TYPE: 登录通道未配置".to_string(),
        });
    };

    // ② js_code → openid（校验微信侧登录态；openid 仅入日志，不落库——每次登录都重新取号）
    let session_data = match code2session(&appid, &secret, b.js_code.trim()).await {
        Ok(d) => d,
        Err(e) => {
            log::warn!("小程序一键登录 jscode2session 失败: {e}");
            return HttpResponse::BadRequest().json(AuthError {
                error: "WX_CODE_INVALID: 微信登录凭证无效或已过期".to_string(),
            });
        }
    };
    let openid = session_data["openid"].as_str().unwrap_or("");

    // ③ phone_code → 明文手机号（数字归一）
    let access_token = match wechat_access_token(&appid, &secret).await {
        Ok(t) => t,
        Err(e) => {
            log::error!("小程序一键登录取 access_token 失败: {e}");
            return HttpResponse::BadGateway().json(AuthError {
                error: "WX_API_ERROR: 微信服务暂不可用，请稍后重试".to_string(),
            });
        }
    };
    let phone = match fetch_phone_number(&access_token, b.phone_code.trim()).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("小程序一键登录 getuserphonenumber 失败: {e}");
            return HttpResponse::BadRequest().json(AuthError {
                error: "WX_CODE_INVALID: 手机号凭证无效或已过期，请重新授权".to_string(),
            });
        }
    };

    // ④ 手机号查司机（数字归一比对；读径 = 门户 canonical 表达式
    //    `portal_write::driver_phone_expr` 的登录侧投影——电话载体 = 联系方式链
    //    （实体→联系人→电话值叶 `zc_id_info-telephone`，default_info 优先、id 最大者），
    //    存量行回退 `comments` 旧段「电话:<号>」（2026-09-24 d78bcfca06 写径迁移后的
    //    回退分支，存量订正完自然失效）。同号多企多命中 → 取最新一条 + warn（用户裁决 2026-09-23）
    let driver = sqlx::query_as::<_, (i64, Option<String>, Option<i64>)>(
        r#"SELECT e.id, e.notice, e.fk_user
             FROM "isahl"."zc_id_empl-natural" e
             JOIN "isahl"."zc_id_subj-org_rr_employee" ore
               ON ore.ref_right = e.id AND ore.deleted_at IS NULL
            WHERE e.deleted_at IS NULL
              AND regexp_replace(COALESCE(
                       (SELECT t.notice
                          FROM "isahl"."zc_id_entity_rr_contacts" rc
                          JOIN "isahl"."zc_id_contacts" c
                            ON c.id = rc.ref_right AND c.deleted_at IS NULL
                          JOIN "isahl"."zc_id_contacts_rr_infos" ri
                            ON ri.ref_left = c.id AND ri.deleted_at IS NULL
                          JOIN "isahl"."zc_id_info-telephone" t
                            ON t.id = ri.ref_right AND t.deleted_at IS NULL
                         WHERE rc.ref_left = e.id AND rc.deleted_at IS NULL
                         ORDER BY ri.default_info DESC NULLS LAST, t.id DESC LIMIT 1),
                       substring(e.comments from '电话[:：]?\s*([0-9+\-]{5,20})'),
                       ''), '[^0-9]', '', 'g') = $1
            ORDER BY e.created_at DESC, e.id DESC
            LIMIT 1"#,
    )
    .bind(&phone)
    .fetch_optional(pool.get_ref())
    .await;
    let (driver_id, driver_name, fk_user) = match driver {
        Ok(Some(d)) => d,
        Ok(None) => {
            log::info!("小程序一键登录: 手机号未命中司机 phone={phone} openid={openid}");
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "DRIVER_NOT_FOUND",
                "message": "未找到司机，请联系调度人员处理",
            }));
        }
        Err(e) => {
            log::error!("小程序一键登录司机查询失败: {e}");
            return HttpResponse::InternalServerError().json(AuthError {
                error: "DB_ERROR".to_string(),
            });
        }
    };
    log::info!(
        "小程序一键登录命中司机: driver={driver_id} name={driver_name:?} phone={phone} openid={openid}"
    );

    // ⑤ fk_user → auth_users（未绑定 → 报错不注册；账号须登记司机时经 user 字段预绑）
    let Some(user_id) = fk_user else {
        log::warn!("小程序一键登录: 司机未绑定登录账号 driver={driver_id}");
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "DRIVER_UNBOUND",
            "message": "司机未绑定登录账号，请联系调度人员处理",
        }));
    };
    let account = sqlx::query_as::<_, (Option<String>, bool, bool)>(
        "SELECT status, is_active, COALESCE(mfa_enabled, false) \
         FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await;
    let (status, is_active, mfa_enabled) = match account {
        Ok(Some(a)) => a,
        Ok(None) => {
            log::warn!(
                "小程序一键登录: fk_user 指向的账号不存在 driver={driver_id} user={user_id}"
            );
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
