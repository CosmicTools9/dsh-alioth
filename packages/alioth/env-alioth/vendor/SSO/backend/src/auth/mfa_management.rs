//! 用户 MFA 自助管理 handlers
//!
//! POST /auth/me/mfa/init    — 生成 TOTP 密钥 + QR code（首次启用）
use actix_web::{web, HttpRequest, HttpResponse};
use base32::Alphabet;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::crypto::{decode_secret, encode_encrypted};
use super::jwt::{decode_token_any, Claims};
use super::login::AuthError;
use super::mfa::{generate_qr_code_image, generate_totp_secret, verify_totp_code};
use super::password::verify_password_async;

#[derive(Debug, Deserialize)]
pub struct MfaEnableRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct MfaDisableRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct MfaStatusResponse {
    pub enabled: bool,
    pub has_secret: bool,
    pub bypass_codes_remaining: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct MfaInitResponse {
    pub secret: String,
    pub qr_code_svg: String,
    pub provisioning_uri: String,
}

/// 从请求中提取 user_id（通过 JWT 验证）
fn extract_user_id(req: &HttpRequest, public_keys: &[&[u8]]) -> Result<i64, HttpResponse> {
    let access_token = match req.cookie("access_token") {
        Some(c) => c.value().to_string(),
        None => match req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|auth| auth.strip_prefix("Bearer "))
        {
            Some(token) => token.to_string(),
            None => {
                return Err(HttpResponse::Unauthorized().json(AuthError {
                    error: "No authentication token".to_string(),
                }))
            }
        },
    };

    let claims: Claims = match decode_token_any(&access_token, public_keys) {
        Ok(c) => c,
        Err(_) => {
            return Err(HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or expired token".to_string(),
            }))
        }
    };

    match claims.sub.parse::<i64>() {
        Ok(id) => Ok(id),
        Err(_) => Err(HttpResponse::Unauthorized().json(AuthError {
            error: "Invalid user ID in token".to_string(),
        })),
    }
}

/// POST /auth/me/mfa/init
///
/// 生成新的 TOTP 密钥 + QR code SVG 字符串。
/// 仅初始化，未启用 — 用户需扫 QR 并提交 enable 才算完成。
pub async fn mfa_init(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<super::AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state.verification_keys()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    let user_email: Option<String> =
        sqlx::query_scalar("SELECT email FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None)
            .flatten();

    let account = user_email.unwrap_or_else(|| format!("user-{}", user_id));
    let setup = match generate_totp_secret(&account, "AliothStudio") {
        Ok(s) => s,
        Err(e) => {
            log::error!("MFA init: failed to generate TOTP: {:?}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to generate MFA setup".to_string(),
            });
        }
    };

    let qr_svg = match generate_qr_code_image(&setup) {
        Ok(bytes) => String::from_utf8(bytes).unwrap_or_default(),
        Err(e) => {
            log::error!("MFA init: failed to generate QR: {:?}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to generate QR code".to_string(),
            });
        }
    };

    // 把 secret 加密后落库（AES-256-GCM），标记 enabled=false，用户 enable 时再置 true。
    let stored_secret = match encode_encrypted(&state.encryption_key, setup.secret.as_bytes()) {
        Ok(s) => s,
        Err(e) => {
            log::error!("MFA init: failed to encrypt secret: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to store MFA secret".to_string(),
            });
        }
    };
    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.auth_users SET mfa_secret = $1, mfa_enabled = false WHERE id = $2",
    )
    .bind(&stored_secret)
    .bind(user_id)
    .execute(pool.get_ref())
    .await
    {
        log::error!("MFA init: failed to store secret: {}", e);
        return HttpResponse::InternalServerError().json(AuthError {
            error: "Failed to store MFA secret".to_string(),
        });
    }

    HttpResponse::Ok().json(MfaInitResponse {
        secret: setup.secret,
        qr_code_svg: qr_svg,
        provisioning_uri: setup.qr_code_data,
    })
}

/// POST /auth/me/mfa/enable
///
/// 用户提交扫了 QR 后的 6 位 TOTP 码。
/// 验证通过则 mfa_enabled=true，生成 bypass 码返回。
pub async fn mfa_enable(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<super::AuthState>,
    body: web::Json<MfaEnableRequest>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state.verification_keys()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    // 读取已有 secret
    let secret_encoded: Option<String> =
        sqlx::query_scalar("SELECT mfa_secret FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None)
            .flatten();

    let secret_encoded = match secret_encoded {
        Some(s) if !s.is_empty() => s,
        _ => {
            return HttpResponse::BadRequest().json(AuthError {
                error: "MFA not initialized. Call /auth/me/mfa/init first.".to_string(),
            })
        }
    };

    // 还原 base32 字符串：支持 enc: 前缀密文（新）与旧明文 base32（迁移期兼容）。
    let base32_secret = match decode_secret(&state.encryption_key, &secret_encoded) {
        Ok(s) => s,
        Err(e) => {
            log::error!("MFA enable: failed to decode stored secret: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Stored MFA secret is corrupted".to_string(),
            });
        }
    };

    let secret_bytes = match base32::decode(Alphabet::Rfc4648 { padding: false }, &base32_secret) {
        Some(b) => b,
        None => {
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Stored MFA secret is corrupted".to_string(),
            })
        }
    };

    if !verify_totp_code(&secret_bytes, &body.code) {
        return HttpResponse::Forbidden().json(AuthError {
            error: "Invalid verification code".to_string(),
        });
    }

    // 生成 5 个 bypass 码
    let bypass = super::mfa::generate_mfa_bypass_codes(5);

    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.auth_users
         SET mfa_enabled = true, mfa_bypass_codes = $1, updated_at = NOW()
         WHERE id = $2",
    )
    .bind(&bypass)
    .bind(user_id)
    .execute(pool.get_ref())
    .await
    {
        log::error!("MFA enable: DB update failed for user {}: {}", user_id, e);
        return HttpResponse::InternalServerError().json(AuthError {
            error: "Failed to enable MFA".to_string(),
        });
    }

    HttpResponse::Ok().json(serde_json::json!({
        "message": "MFA enabled successfully",
        "bypass_codes": bypass,
        "warning": "Save these bypass codes — they are shown only once!"
    }))
}

/// POST /auth/me/mfa/disable
///
/// 关闭 MFA。需要密码二次确认。
pub async fn mfa_disable(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<super::AuthState>,
    body: web::Json<MfaDisableRequest>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state.verification_keys()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    let password_hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None)
            .flatten();

    let password_hash = match password_hash {
        Some(h) => h,
        None => {
            return HttpResponse::BadRequest().json(AuthError {
                error: "No password set for this account".to_string(),
            })
        }
    };

    match verify_password_async(body.password.clone(), password_hash).await {
        Ok(Some(_)) => {} // password matches
        Ok(None) => {
            return HttpResponse::Forbidden().json(AuthError {
                error: "Password is incorrect".to_string(),
            })
        }
        Err(e) => {
            log::error!("MFA disable: password verify failed: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to verify password".to_string(),
            });
        }
    }

    // 清空 MFA 状态
    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.auth_users
         SET mfa_enabled = false, mfa_secret = NULL, mfa_bypass_codes = NULL, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(pool.get_ref())
    .await
    {
        log::error!("MFA disable: DB update failed: {}", e);
        return HttpResponse::InternalServerError().json(AuthError {
            error: "Failed to disable MFA".to_string(),
        });
    }

    HttpResponse::Ok().json(serde_json::json!({
        "message": "MFA disabled successfully"
    }))
}

/// GET /auth/me/mfa/status
pub async fn mfa_status(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<super::AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state.verification_keys()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    let row: Option<(bool, Option<String>, Option<Vec<String>>)> = sqlx::query_as(
        "SELECT mfa_enabled, mfa_secret, mfa_bypass_codes
         FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    match row {
        Some((enabled, secret, bypass)) => HttpResponse::Ok().json(MfaStatusResponse {
            enabled,
            has_secret: secret.map(|s| !s.is_empty()).unwrap_or(false),
            bypass_codes_remaining: bypass.map(|v| v.len()),
        }),
        None => HttpResponse::NotFound().json(AuthError {
            error: "User not found".to_string(),
        }),
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth/me/mfa")
            .route("/init", web::post().to(mfa_init))
            .route("/enable", web::post().to(mfa_enable))
            .route("/disable", web::post().to(mfa_disable))
            .route("/status", web::get().to(mfa_status)),
    );
}
