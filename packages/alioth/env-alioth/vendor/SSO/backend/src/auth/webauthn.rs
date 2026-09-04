//! WebAuthn / Passkey 注册与认证（无密码登录）
//!
//! 使用 webauthn-rs 处理 challenge / attestation / assertion 校验。
//! 凭据（Passkey / SecurityKey 的序列化体）与 begin/complete 之间的 challenge 状态
//! 持久化到 `isahl_auth` schema，绝不存储私钥。
//!
//! 说明：当前 webauthn-rs 0.5 的 `finish_*_authentication` 必须在 begin 阶段已知用户
//! （凭据列表来自 begin 时传入的凭据），因此登录流程需要邮箱定位用户；浏览器侧的
//! 平台 passkey 仍走可发现凭据 UX，但服务端按用户解析。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use webauthn_rs::prelude::*;

use super::jwt::{
    decode_token_any, encode_access_token, encode_refresh_token, set_access_cookie,
    set_refresh_cookie, Claims,
};
use super::login::AuthError;
use super::session::{CreateSessionRequest, SessionManager};
use super::AuthState;

/// challenge 状态 TTL（秒），与提案一致：5 分钟
const CHALLENGE_TTL_SECS: i64 = 300;

/// 凭据类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialKind {
    Passkey,
    SecurityKey,
}

impl CredentialKind {
    fn as_str(self) -> &'static str {
        match self {
            CredentialKind::Passkey => "passkey",
            CredentialKind::SecurityKey => "security_key",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "passkey" => Some(CredentialKind::Passkey),
            "security_key" => Some(CredentialKind::SecurityKey),
            _ => None,
        }
    }
}

/// begin/complete 之间持久化的 challenge 用途（同时也决定反序列化类型）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChallengePurpose {
    RegisterPasskey,
    RegisterSecurityKey,
    LoginPasskey,
    LoginSecurityKey,
}

impl ChallengePurpose {
    fn as_str(self) -> &'static str {
        match self {
            ChallengePurpose::RegisterPasskey => "register_passkey",
            ChallengePurpose::RegisterSecurityKey => "register_securitykey",
            ChallengePurpose::LoginPasskey => "login_passkey",
            ChallengePurpose::LoginSecurityKey => "login_securitykey",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "register_passkey" => Some(ChallengePurpose::RegisterPasskey),
            "register_securitykey" => Some(ChallengePurpose::RegisterSecurityKey),
            "login_passkey" => Some(ChallengePurpose::LoginPasskey),
            "login_securitykey" => Some(ChallengePurpose::LoginSecurityKey),
            _ => None,
        }
    }
}

/// 注册 / 登录 begin 请求体
#[derive(Debug, Deserialize)]
pub struct WebauthnBeginRequest {
    /// 注册时：凭据类型（默认 passkey）
    #[serde(default)]
    pub credential_type: Option<String>,
    /// 登录时：用户邮箱，用于定位该用户的凭据（当前库版本要求已知用户）
    #[serde(default)]
    pub email: Option<String>,
    /// 注册时：设备名（展示用）
    #[serde(default)]
    pub device_name: Option<String>,
}

/// 凭据列表项
#[derive(Debug, Serialize)]
pub struct CredentialListItem {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub credential_id: String,
    pub credential_type: String,
    pub transports: Vec<String>,
    pub device_name: Option<String>,
    #[serde(with = "common::serde_zuid")]
    pub sign_count: i64,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

/// 注册 / 登录状态包装（用于携带 device_name 等元信息）
#[derive(Debug, Serialize, Deserialize)]
struct StoredRegState<T> {
    device_name: Option<String>,
    data: T,
}

// ----------------------------------------------------------------------------
// 请求辅助
// ----------------------------------------------------------------------------

fn b64url(data: &[u8]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.encode(data)
}

fn unauthorized(error: &str) -> HttpResponse {
    HttpResponse::Unauthorized().json(AuthError {
        error: error.to_string(),
    })
}

fn bad_request(error: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(AuthError {
        error: error.to_string(),
    })
}

/// 从请求中提取并校验已登录用户 ID（复用 profile 的 token 解析模式）
fn extract_user_id(req: &HttpRequest, state: &AuthState) -> Result<i64, HttpResponse> {
    let token = req
        .cookie("access_token")
        .map(|c| c.value().to_string())
        .or_else(|| {
            req.headers()
                .get(actix_web::http::header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|a| a.strip_prefix("Bearer ").map(|t| t.to_string()))
        });

    let token = match token {
        Some(t) => t,
        None => return Err(unauthorized("No authentication token")),
    };

    let claims = match decode_token_any(&token, &state.verification_keys()) {
        Ok(c) => c,
        Err(_) => return Err(unauthorized("Invalid or expired token")),
    };

    claims
        .sub
        .parse::<i64>()
        .map_err(|_| unauthorized("Invalid user ID in token"))
}

/// 依据请求推导 Webauthn 实例（RP ID 取 SSO_DOMAIN 或请求 host；origin 取请求 scheme+host）
fn build_webauthn(req: &HttpRequest) -> Result<Webauthn, HttpResponse> {
    let conn = req.connection_info();
    let host = conn.host().to_string();
    let scheme = conn.scheme().to_string();
    let rp_id = std::env::var("SSO_DOMAIN")
        .ok()
        .unwrap_or_else(|| host.split(':').next().unwrap_or("localhost").to_string());
    let origin = Url::parse(&format!("{}://{}", scheme, host)).map_err(|_| {
        HttpResponse::InternalServerError().json(AuthError {
            error: "Invalid origin".into(),
        })
    })?;

    WebauthnBuilder::new(&rp_id, &origin)
        .map_err(|_| {
            HttpResponse::InternalServerError().json(AuthError {
                error: "Invalid webauthn configuration".into(),
            })
        })?
        .rp_name("Alioth Studio")
        .build()
        .map_err(|_| {
            HttpResponse::InternalServerError().json(AuthError {
                error: "Invalid webauthn configuration".into(),
            })
        })
}

fn extract_transports<T: Serialize>(transports: &Option<Vec<T>>) -> Vec<String> {
    transports
        .as_ref()
        .map(|list| {
            list.iter()
                .filter_map(|t| {
                    serde_json::to_string(t)
                        .ok()
                        .map(|s| s.trim_matches('"').to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

// ----------------------------------------------------------------------------
// challenge 状态持久化
// ----------------------------------------------------------------------------

async fn store_challenge(
    pool: &PgPool,
    challenge: &str,
    user_id: i64,
    purpose: ChallengePurpose,
    state: &str,
) -> Result<(), sqlx::Error> {
    let expires = Utc::now() + TimeDelta::seconds(CHALLENGE_TTL_SECS);
    sqlx::query(
        "INSERT INTO isahl_auth.webauthn_challenges (challenge, user_id, purpose, state, expires_at) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (challenge) DO UPDATE SET user_id = $2, purpose = $3, state = $4, expires_at = $5",
    )
    .bind(challenge)
    .bind(user_id)
    .bind(purpose.as_str())
    .bind(state)
    .bind(expires)
    .execute(pool)
    .await?;
    Ok(())
}

/// 读取 challenge 状态（并校验未过期）。返回 (user_id, purpose, state_json)
async fn take_challenge(
    pool: &PgPool,
    challenge: &str,
) -> Result<Option<(i64, String, String)>, sqlx::Error> {
    let row: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT user_id, purpose, state FROM isahl_auth.webauthn_challenges \
         WHERE challenge = $1 AND expires_at > NOW()",
    )
    .bind(challenge)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

async fn delete_challenge(pool: &PgPool, challenge: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM isahl_auth.webauthn_challenges WHERE challenge = $1")
        .bind(challenge)
        .execute(pool)
        .await?;
    Ok(())
}

async fn lookup_user_by_email(pool: &PgPool, email: &str) -> Result<Option<i64>, sqlx::Error> {
    let id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE lower(email) = lower($1)")
            .bind(email)
            .fetch_optional(pool)
            .await?;
    Ok(id)
}

/// 加载用户凭据（kind + 序列化 blob）
async fn load_user_credentials(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<(CredentialKind, Vec<u8>)>, sqlx::Error> {
    let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT credential_type, public_key_cose FROM isahl_auth.webauthn_credentials \
         WHERE user_id = $1 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(ct, blob)| CredentialKind::from_str(&ct).map(|k| (k, blob)))
        .collect())
}

async fn lookup_credential_by_id(
    pool: &PgPool,
    cred_id: &[u8],
) -> Result<Option<(i64, CredentialKind, Vec<u8>)>, sqlx::Error> {
    let row: Option<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT user_id, credential_type, public_key_cose FROM isahl_auth.webauthn_credentials \
         WHERE credential_id = $1 AND deleted_at IS NULL",
    )
    .bind(cred_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(uid, ct, blob)| CredentialKind::from_str(&ct).map(|k| (uid, k, blob))))
}

async fn credential_exists(pool: &PgPool, cred_id: &[u8]) -> Result<bool, sqlx::Error> {
    let exists: Option<bool> = sqlx::query_scalar(
        "SELECT TRUE FROM isahl_auth.webauthn_credentials WHERE credential_id = $1 AND deleted_at IS NULL",
    )
    .bind(cred_id)
    .fetch_optional(pool)
    .await?;
    Ok(exists.is_some())
}

// ----------------------------------------------------------------------------
// 处理器
// ----------------------------------------------------------------------------

/// 注册开始：返回 PublicKeyCredentialCreationOptions；状态存入 webauthn_challenges
pub async fn register_begin(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    body: web::Json<WebauthnBeginRequest>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, state.get_ref()) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let webauthn = match build_webauthn(&req) {
        Ok(w) => w,
        Err(resp) => return resp,
    };

    // 稳定的 per-user UUID（仅用于嵌入凭据 user.id，无需持久化）
    let user_uuid = Uuid::from_u128(user_id as u128);
    let kind = body
        .credential_type
        .as_deref()
        .and_then(CredentialKind::from_str)
        .unwrap_or(CredentialKind::Passkey);

    let result = match kind {
        CredentialKind::Passkey => webauthn
            .start_passkey_registration(user_uuid, &user_id.to_string(), &user_id.to_string(), None)
            .map(|(ccr, st)| {
                (
                    ChallengePurpose::RegisterPasskey,
                    ccr,
                    serde_json::to_string(&StoredRegState {
                        device_name: body.device_name.clone(),
                        data: st,
                    })
                    .unwrap(),
                )
            }),
        CredentialKind::SecurityKey => webauthn
            .start_securitykey_registration(
                user_uuid,
                &user_id.to_string(),
                &user_id.to_string(),
                None,
                None,
                None,
            )
            .map(|(ccr, st)| {
                (
                    ChallengePurpose::RegisterSecurityKey,
                    ccr,
                    serde_json::to_string(&StoredRegState {
                        device_name: body.device_name.clone(),
                        data: st,
                    })
                    .unwrap(),
                )
            }),
    };

    let (purpose, ccr, state_json) = match result {
        Ok(r) => r,
        Err(e) => return bad_request(&format!("Failed to start registration: {}", e)),
    };

    let challenge = b64url(ccr.public_key.challenge.as_ref());
    if let Err(e) = store_challenge(pool.get_ref(), &challenge, user_id, purpose, &state_json).await
    {
        log::error!("Failed to store webauthn challenge: {}", e);
        return HttpResponse::InternalServerError().json(AuthError {
            error: "Failed to store challenge".into(),
        });
    }

    HttpResponse::Ok().json(ccr)
}

/// 注册完成：校验 attestation，存储凭据
pub async fn register_complete(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
    reg: web::Json<RegisterPublicKeyCredential>,
) -> HttpResponse {
    let webauthn = match build_webauthn(&req) {
        Ok(w) => w,
        Err(resp) => return resp,
    };

    // 从 client_data_json 提取 challenge 以定位 begin 时存储的状态
    let challenge_lookup = match extract_challenge_from_client_data(&reg.response.client_data_json)
    {
        Some(c) => c,
        None => return bad_request("Invalid client data"),
    };

    let (user_id, purpose_str, state_json) =
        match take_challenge(pool.get_ref(), &challenge_lookup).await {
            Ok(Some(r)) => r,
            Ok(None) => return bad_request("Unknown or expired challenge"),
            Err(e) => {
                log::error!("Failed to load challenge: {}", e);
                return HttpResponse::InternalServerError().json(AuthError {
                    error: "Failed to load challenge".into(),
                });
            }
        };

    let purpose = match ChallengePurpose::from_str(&purpose_str) {
        Some(p) => p,
        None => return bad_request("Invalid challenge purpose"),
    };

    match purpose {
        ChallengePurpose::RegisterPasskey => {
            let stored: StoredRegState<PasskeyRegistration> =
                match serde_json::from_str(&state_json) {
                    Ok(s) => s,
                    Err(_) => return bad_request("Corrupt registration state"),
                };
            let passkey = match webauthn.finish_passkey_registration(&reg, &stored.data) {
                Ok(p) => p,
                Err(e) => return bad_request(&format!("Registration failed: {}", e)),
            };
            let cred_id = passkey.cred_id().as_ref().to_vec();
            let transports = extract_transports(&reg.response.transports);
            let exists = match credential_exists(pool.get_ref(), &cred_id).await {
                Ok(b) => b,
                Err(e) => {
                    log::error!("Failed to check credential: {}", e);
                    return HttpResponse::InternalServerError().json(AuthError {
                        error: "Failed to check credential".into(),
                    });
                }
            };
            if exists {
                return HttpResponse::Conflict().json(AuthError {
                    error: "Credential already registered".into(),
                });
            }
            let blob = match serde_json::to_vec(&passkey) {
                Ok(b) => b,
                Err(_) => {
                    return HttpResponse::InternalServerError().json(AuthError {
                        error: "Failed to serialize credential".into(),
                    })
                }
            };
            if let Err(e) = insert_credential(
                pool.get_ref(),
                user_id,
                &cred_id,
                &blob,
                CredentialKind::Passkey,
                &transports,
                &stored.device_name,
            )
            .await
            {
                log::error!("Failed to store credential: {}", e);
                return HttpResponse::InternalServerError().json(AuthError {
                    error: "Failed to store credential".into(),
                });
            }
        }
        ChallengePurpose::RegisterSecurityKey => {
            let stored: StoredRegState<SecurityKeyRegistration> =
                match serde_json::from_str(&state_json) {
                    Ok(s) => s,
                    Err(_) => return bad_request("Corrupt registration state"),
                };
            let sk = match webauthn.finish_securitykey_registration(&reg, &stored.data) {
                Ok(s) => s,
                Err(e) => return bad_request(&format!("Registration failed: {}", e)),
            };
            let cred_id = sk.cred_id().as_ref().to_vec();
            let transports = extract_transports(&reg.response.transports);
            let exists = match credential_exists(pool.get_ref(), &cred_id).await {
                Ok(b) => b,
                Err(e) => {
                    log::error!("Failed to check credential: {}", e);
                    return HttpResponse::InternalServerError().json(AuthError {
                        error: "Failed to check credential".into(),
                    });
                }
            };
            if exists {
                return HttpResponse::Conflict().json(AuthError {
                    error: "Credential already registered".into(),
                });
            }
            let blob = match serde_json::to_vec(&sk) {
                Ok(b) => b,
                Err(_) => {
                    return HttpResponse::InternalServerError().json(AuthError {
                        error: "Failed to serialize credential".into(),
                    })
                }
            };
            if let Err(e) = insert_credential(
                pool.get_ref(),
                user_id,
                &cred_id,
                &blob,
                CredentialKind::SecurityKey,
                &transports,
                &stored.device_name,
            )
            .await
            {
                log::error!("Failed to store credential: {}", e);
                return HttpResponse::InternalServerError().json(AuthError {
                    error: "Failed to store credential".into(),
                });
            }
        }
        _ => return bad_request("Challenge purpose mismatch"),
    };

    let _ = delete_challenge(pool.get_ref(), &challenge_lookup).await;

    HttpResponse::Ok().json(serde_json::json!({ "verified": true }))
}

/// 登录开始：返回 PublicKeyCredentialRequestOptions；状态存入 webauthn_challenges
pub async fn login_begin(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
    body: web::Json<WebauthnBeginRequest>,
) -> HttpResponse {
    let webauthn = match build_webauthn(&req) {
        Ok(w) => w,
        Err(resp) => return resp,
    };

    let email = match &body.email {
        Some(e) => e.clone(),
        None => {
            return bad_request("Email is required to begin passkey authentication (this library requires a known user)")
        }
    };

    let user_id = match lookup_user_by_email(pool.get_ref(), &email).await {
        Ok(Some(id)) => id,
        Ok(None) => return bad_request("Unknown user"),
        Err(e) => {
            log::error!("Failed to lookup user: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to lookup user".into(),
            });
        }
    };

    let creds = match load_user_credentials(pool.get_ref(), user_id).await {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to load credentials: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to load credentials".into(),
            });
        }
    };

    if creds.is_empty() {
        return bad_request("No passkeys registered for this user");
    }

    let (purpose, rcr, state_json) = match build_login_options(&webauthn, &creds) {
        Ok(r) => r,
        Err(e) => return bad_request(&format!("Failed to start authentication: {}", e)),
    };

    let challenge = b64url(rcr.public_key.challenge.as_ref());
    if let Err(e) = store_challenge(pool.get_ref(), &challenge, user_id, purpose, &state_json).await
    {
        log::error!("Failed to store webauthn challenge: {}", e);
        return HttpResponse::InternalServerError().json(AuthError {
            error: "Failed to store challenge".into(),
        });
    }

    HttpResponse::Ok().json(rcr)
}

/// 登录完成：校验 assertion，签发会话 + access/refresh token
pub async fn login_complete(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    pk: web::Json<PublicKeyCredential>,
) -> HttpResponse {
    let webauthn = match build_webauthn(&req) {
        Ok(w) => w,
        Err(resp) => return resp,
    };

    let challenge_lookup = match extract_challenge_from_client_data(&pk.response.client_data_json) {
        Some(c) => c,
        None => return bad_request("Invalid client data"),
    };

    let (user_id, purpose_str, state_json) =
        match take_challenge(pool.get_ref(), &challenge_lookup).await {
            Ok(Some(r)) => r,
            Ok(None) => return bad_request("Unknown or expired challenge"),
            Err(e) => {
                log::error!("Failed to load challenge: {}", e);
                return HttpResponse::InternalServerError().json(AuthError {
                    error: "Failed to load challenge".into(),
                });
            }
        };

    let purpose = match ChallengePurpose::from_str(&purpose_str) {
        Some(p) => p,
        None => return bad_request("Invalid challenge purpose"),
    };

    // 校验 assertion，得到断言结果（含 cred_id 与计数器）
    let auth_result = match purpose {
        ChallengePurpose::LoginPasskey => {
            let st: PasskeyAuthentication = match serde_json::from_str(&state_json) {
                Ok(s) => s,
                Err(_) => return bad_request("Corrupt authentication state"),
            };
            match webauthn.finish_passkey_authentication(&pk, &st) {
                Ok(r) => r,
                Err(e) => return bad_request(&format!("Authentication failed: {}", e)),
            }
        }
        ChallengePurpose::LoginSecurityKey => {
            let st: SecurityKeyAuthentication = match serde_json::from_str(&state_json) {
                Ok(s) => s,
                Err(_) => return bad_request("Corrupt authentication state"),
            };
            match webauthn.finish_securitykey_authentication(&pk, &st) {
                Ok(r) => r,
                Err(e) => return bad_request(&format!("Authentication failed: {}", e)),
            }
        }
        _ => return bad_request("Challenge purpose mismatch"),
    };

    let cred_id = auth_result.cred_id().as_ref().to_vec();

    // 定位凭据所属用户（discovery / 已知用户均以此为准）
    let (owner_id, cred_kind, blob) = match lookup_credential_by_id(pool.get_ref(), &cred_id).await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            let _ = delete_challenge(pool.get_ref(), &challenge_lookup).await;
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Unknown or revoked credential".into(),
            });
        }
        Err(e) => {
            log::error!("Failed to lookup credential: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to lookup credential".into(),
            });
        }
    };

    // 更新计数器并回写凭据（防重放 / 克隆检测）
    // 安全：凭据归属必须与 begin 阶段定位的用户一致
    if owner_id != user_id {
        let _ = delete_challenge(pool.get_ref(), &challenge_lookup).await;
        return unauthorized("Credential does not belong to the expected user");
    }

    let new_counter = auth_result.counter() as i64;
    let updated_blob = match cred_kind {
        CredentialKind::Passkey => {
            let mut p: Passkey = match serde_json::from_slice(&blob) {
                Ok(p) => p,
                Err(_) => return bad_request("Corrupt credential"),
            };
            p.update_credential(&auth_result);
            match serde_json::to_vec(&p) {
                Ok(b) => b,
                Err(_) => blob,
            }
        }
        CredentialKind::SecurityKey => {
            let mut sk: SecurityKey = match serde_json::from_slice(&blob) {
                Ok(s) => s,
                Err(_) => return bad_request("Corrupt credential"),
            };
            sk.update_credential(&auth_result);
            match serde_json::to_vec(&sk) {
                Ok(b) => b,
                Err(_) => blob,
            }
        }
    };

    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.webauthn_credentials \
         SET public_key_cose = $1, sign_count = $2, last_used_at = NOW() \
         WHERE credential_id = $3 AND deleted_at IS NULL",
    )
    .bind(&updated_blob)
    .bind(new_counter)
    .bind(&cred_id)
    .execute(pool.get_ref())
    .await
    {
        log::error!("Failed to update credential: {}", e);
        // 不阻断登录，仅告警
    }

    let _ = delete_challenge(pool.get_ref(), &challenge_lookup).await;

    issue_session_and_tokens(&req, pool.get_ref(), state.get_ref(), owner_id).await
}

/// 列出当前用户的凭据
pub async fn list_credentials(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, state.get_ref()) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let rows = sqlx::query(
        "SELECT id, credential_id, credential_type, transports, device_name, sign_count, last_used_at, created_at \
         FROM isahl_auth.webauthn_credentials WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to list credentials: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to list credentials".into(),
            });
        }
    };

    let items: Vec<CredentialListItem> = rows
        .into_iter()
        .map(|row| {
            let transports_raw: String = row
                .try_get("transports")
                .unwrap_or_else(|_| "[]".to_string());
            let transports: Vec<String> = serde_json::from_str(&transports_raw).unwrap_or_default();
            let cid: Vec<u8> = row.try_get("credential_id").unwrap_or_default();
            CredentialListItem {
                id: row.try_get("id").unwrap_or(0),
                credential_id: b64url(&cid),
                credential_type: row.try_get("credential_type").unwrap_or_default(),
                transports,
                device_name: row.try_get("device_name").ok(),
                sign_count: row.try_get("sign_count").unwrap_or(0),
                last_used_at: row
                    .try_get::<chrono::DateTime<chrono::Utc>, _>("last_used_at")
                    .ok()
                    .map(|t| t.to_rfc3339()),
                created_at: row
                    .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default(),
            }
        })
        .collect();

    HttpResponse::Ok().json(serde_json::json!({ "credentials": items }))
}

/// 删除（吊销）凭据
pub async fn delete_credential(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, state.get_ref()) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let cred_id = path.into_inner();

    let result = sqlx::query(
        "UPDATE isahl_auth.webauthn_credentials SET deleted_at = NOW() \
         WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
    )
    .bind(cred_id)
    .bind(user_id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({ "deleted": true }))
        }
        Ok(_) => HttpResponse::NotFound().json(AuthError {
            error: "Credential not found".into(),
        }),
        Err(e) => {
            log::error!("Failed to delete credential: {}", e);
            HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to delete credential".into(),
            })
        }
    }
}

// ----------------------------------------------------------------------------
// 内部辅助
// ----------------------------------------------------------------------------

fn extract_challenge_from_client_data(client_data_json: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(client_data_json).ok()?;
    v.get("challenge")?.as_str().map(|s| s.to_string())
}

fn build_login_options(
    webauthn: &Webauthn,
    creds: &[(CredentialKind, Vec<u8>)],
) -> Result<(ChallengePurpose, RequestChallengeResponse, String), WebauthnError> {
    let passkeys: Vec<Passkey> = creds
        .iter()
        .filter_map(|(k, blob)| {
            if *k == CredentialKind::Passkey {
                serde_json::from_slice(blob).ok()
            } else {
                None
            }
        })
        .collect();

    if !passkeys.is_empty() {
        let (rcr, st) = webauthn.start_passkey_authentication(&passkeys)?;
        return Ok((
            ChallengePurpose::LoginPasskey,
            rcr,
            serde_json::to_string(&st).unwrap(),
        ));
    }

    let security_keys: Vec<SecurityKey> = creds
        .iter()
        .filter_map(|(k, blob)| {
            if *k == CredentialKind::SecurityKey {
                serde_json::from_slice(blob).ok()
            } else {
                None
            }
        })
        .collect();

    let (rcr, st) = webauthn.start_securitykey_authentication(&security_keys)?;
    Ok((
        ChallengePurpose::LoginSecurityKey,
        rcr,
        serde_json::to_string(&st).unwrap(),
    ))
}

async fn insert_credential(
    pool: &PgPool,
    user_id: i64,
    cred_id: &[u8],
    blob: &[u8],
    kind: CredentialKind,
    transports: &[String],
    device_name: &Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO isahl_auth.webauthn_credentials \
         (user_id, credential_id, public_key_cose, sign_count, credential_type, transports, device_name) \
         VALUES ($1, $2, $3, 0, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(cred_id)
    .bind(blob)
    .bind(kind.as_str())
    .bind(serde_json::to_string(transports).unwrap_or_else(|_| "[]".to_string()))
    .bind(device_name)
    .execute(pool)
    .await?;
    Ok(())
}

fn get_client_ip(req: &HttpRequest) -> Option<String> {
    req.connection_info()
        .realip_remote_addr()
        .map(|s| s.to_string())
}

fn get_user_agent(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
}

fn hash_refresh_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

async fn store_refresh_token(
    pool: &PgPool,
    user_id: i64,
    token: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    let token_hash = hash_refresh_token(token);
    sqlx::query(
        "INSERT INTO isahl_auth.refresh_tokens (user_id, token_hash, expires_at, created_at) \
         VALUES ($1, $2, $3, NOW()) \
         ON CONFLICT (token_hash) DO UPDATE SET expires_at = $3",
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// 创建会话并签发 access/refresh token（与 password 登录一致）
async fn issue_session_and_tokens(
    req: &HttpRequest,
    pool: &PgPool,
    state: &AuthState,
    user_id: i64,
) -> HttpResponse {
    let session_manager = SessionManager::new((*pool).clone());
    let session = match session_manager
        .create_session(CreateSessionRequest {
            user_id,
            ip_address: get_client_ip(req),
            user_agent: get_user_agent(req),
            ..Default::default()
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create session: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to create session".into(),
            });
        }
    };

    let mut claims = Claims::with_expiry_seconds(
        &user_id.to_string(),
        "",
        false,
        state.jwt_access_expiry_secs,
    );
    claims.sid = session.session_token.clone();

    let access_token = match encode_access_token(&claims, &state.jwt_private_key) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to generate token".into(),
            })
        }
    };

    let refresh_token = match encode_refresh_token(
        &claims,
        &state.jwt_private_key,
        state.jwt_refresh_expiry_secs,
    ) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to generate refresh token".into(),
            })
        }
    };

    let expires_at = Utc::now() + TimeDelta::seconds(state.jwt_refresh_expiry_secs);
    if let Err(e) = store_refresh_token(pool, user_id, &refresh_token, expires_at).await {
        log::error!("Failed to store refresh token: {}", e);
    }

    let response = HttpResponse::Ok().json(super::login::LoginResponse {
        access_token: Some(access_token.clone()),
        refresh_token: Some(refresh_token.clone()),
        mfa_required: false,
        message: Some("Passkey login successful".into()),
        session_id: Some(session.session_token.clone()),
    });

    let response = set_access_cookie(response, &access_token, state.jwt_access_expiry_secs);
    set_refresh_cookie(response, &refresh_token, state.jwt_refresh_expiry_secs)
}

/// 路由注册
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/webauthn/register/begin", web::post().to(register_begin))
        .route(
            "/webauthn/register/complete",
            web::post().to(register_complete),
        )
        .route("/webauthn/login/begin", web::post().to(login_begin))
        .route("/webauthn/login/complete", web::post().to(login_complete))
        .route("/webauthn/credentials", web::get().to(list_credentials))
        .route(
            "/webauthn/credentials/{id}",
            web::delete().to(delete_credential),
        );
}
