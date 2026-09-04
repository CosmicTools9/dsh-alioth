//! ZChat / MQTT unified authentication bridge
//!
//! Provides HTTP endpoints for ZChat clients to obtain and verify JWT tokens.
//! Tokens are issued by SSO and validated by r-player during ZChat session
//! establishment (single handshake, no per-frame token carrying).

use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::{
    jwt::{decode_token_any, encode_access_token, Claims},
    password::verify_password_async,
    AuthState,
};

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ZchatAuthRequest {
    pub grant_type: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ZchatAuthResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
pub struct ZchatVerifyRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct ZchatVerifyResponse {
    pub valid: bool,
    pub sub: Option<String>,
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ZchatErrorResponse {
    pub error: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /auth/zchat
///
/// Supported grant types:
/// - `password`: username + password
/// - `refresh_token`: existing refresh token
pub async fn zchat_auth(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    body: web::Json<ZchatAuthRequest>,
) -> HttpResponse {
    match body.grant_type.as_str() {
        "password" => handle_password_grant(pool, state, body.into_inner()).await,
        "refresh_token" => handle_refresh_grant(pool, state, body.into_inner()).await,
        _ => HttpResponse::BadRequest().json(ZchatErrorResponse {
            error: format!("Unsupported grant_type: {}", body.grant_type),
        }),
    }
}

/// POST /auth/zchat/verify
///
/// Validates a ZChat access token and returns claim details.
pub async fn zchat_verify(
    state: web::Data<AuthState>,
    body: web::Json<ZchatVerifyRequest>,
) -> HttpResponse {
    match decode_token_any(&body.token, &state.verification_keys()) {
        Ok(claims) => HttpResponse::Ok().json(ZchatVerifyResponse {
            valid: true,
            sub: Some(claims.sub),
            protocol: if claims.protocol.is_empty() {
                None
            } else {
                Some(claims.protocol)
            },
            error: None,
        }),
        Err(e) => HttpResponse::Ok().json(ZchatVerifyResponse {
            valid: false,
            sub: None,
            protocol: None,
            error: Some(format!("{e}")),
        }),
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

async fn handle_password_grant(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    body: ZchatAuthRequest,
) -> HttpResponse {
    let username = match body.username {
        Some(u) => u,
        None => {
            return HttpResponse::BadRequest().json(ZchatErrorResponse {
                error: "username is required for password grant".to_string(),
            });
        }
    };
    let password = match body.password {
        Some(p) => p,
        None => {
            return HttpResponse::BadRequest().json(ZchatErrorResponse {
                error: "password is required for password grant".to_string(),
            });
        }
    };

    // Fetch user by email/username
    let query =
        "SELECT id, password_hash, email FROM isahl_auth.auth_users WHERE email = $1 OR username = $1";

    let user_result = sqlx::query_as::<_, (i64, String, Option<String>)>(query)
        .bind(&username)
        .fetch_optional(pool.get_ref())
        .await;

    let (user_id, stored_hash, email) = match user_result {
        Ok(Some(u)) => u,
        Ok(None) => {
            return HttpResponse::Unauthorized().json(ZchatErrorResponse {
                error: "Invalid credentials".to_string(),
            });
        }
        Err(e) => {
            log::error!("ZChat auth DB error: {}", e);
            return HttpResponse::InternalServerError().json(ZchatErrorResponse {
                error: "Internal server error".to_string(),
            });
        }
    };

    // Verify password (offload CPU-intensive Argon2 to blocking pool)
    match verify_password_async(password, stored_hash).await {
        Ok(Some(_)) => {
            // Success
        }
        Ok(None) => {
            return HttpResponse::Unauthorized().json(ZchatErrorResponse {
                error: "Invalid credentials".to_string(),
            });
        }
        Err(e) => {
            log::error!("ZChat auth password verify error: {}", e);
            return HttpResponse::InternalServerError().json(ZchatErrorResponse {
                error: "Internal server error".to_string(),
            });
        }
    };

    // 治理闭环：创建持久化 SsoSession 并绑定到访问令牌 sid，
    // 使 zchat 令牌可被 logout/管理员 revoke 会话吊销（Gateway PEP 会话检查依赖 sid）。
    let session_manager = super::session::SessionManager::new(pool.get_ref().clone());
    let session = match session_manager
        .create_session(super::session::CreateSessionRequest {
            user_id,
            ..Default::default()
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("ZChat auth session create error: {}", e);
            return HttpResponse::InternalServerError().json(ZchatErrorResponse {
                error: "Failed to create session".to_string(),
            });
        }
    };

    // 用 Claims 构造器 + encode_access_token：iss/aud 由 stamp_audience 自动盖章
    // （与 SSO configure_token_validation 绑定值一致，可过 Gateway PEP 令牌绑定）。
    let mut claims = Claims::with_expiry_seconds(
        &user_id.to_string(),
        email.as_deref().unwrap_or(""),
        true,
        state.jwt_access_expiry_secs,
    );
    claims.protocol = "zchat".to_string();
    claims.sid = session.session_token.clone();

    let access_token = match encode_access_token(&claims, &state.jwt_private_key) {
        Ok(t) => t,
        Err(e) => {
            log::error!("ZChat auth token encode error: {}", e);
            return HttpResponse::InternalServerError().json(ZchatErrorResponse {
                error: "Failed to generate token".to_string(),
            });
        }
    };

    HttpResponse::Ok().json(ZchatAuthResponse {
        access_token,
        token_type: "ZChat".to_string(),
        expires_in: state.jwt_access_expiry_secs.max(0) as u64,
    })
}

async fn handle_refresh_grant(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    body: ZchatAuthRequest,
) -> HttpResponse {
    let refresh_token = match body.refresh_token {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest().json(ZchatErrorResponse {
                error: "refresh_token is required for refresh_token grant".to_string(),
            });
        }
    };

    // 1) 签名 + exp + iss/aud 绑定校验（decode_token 强制全部三项；
    //    替换旧的 validate_exp=false 裸解码——过期令牌不再被接受）。
    let refresh_claims = match decode_token_any(&refresh_token, &state.verification_keys()) {
        Ok(claims) => claims,
        Err(e) => {
            return HttpResponse::Unauthorized().json(ZchatErrorResponse {
                error: format!("Invalid refresh token: {e}"),
            });
        }
    };

    // 2) DB 哈希 + 吊销 + 过期校验（与 /auth/refresh 同一实现，经 auth/mod.rs 导出复用）。
    match super::is_valid_refresh_token(pool.get_ref(), &refresh_token).await {
        Ok(true) => {}
        Ok(false) => {
            log::warn!("ZChat refresh: invalid, revoked or expired refresh token");
            return HttpResponse::Unauthorized().json(ZchatErrorResponse {
                error: "Invalid or revoked refresh token".to_string(),
            });
        }
        Err(e) => {
            log::error!("ZChat refresh token DB validation error: {}", e);
            return HttpResponse::InternalServerError().json(ZchatErrorResponse {
                error: "Token validation failed".to_string(),
            });
        }
    }

    // 3) 会话存活校验：sid 非空时强制校验（覆盖「管理员仅 revoke 会话」的吊销路径——
    //    管理员 revoke_user_session 只改 sso_sessions.status，不改 refresh_tokens）。
    if !refresh_claims.sid.is_empty() {
        let session_manager = super::session::SessionManager::new(pool.get_ref().clone());
        match session_manager.validate_session(&refresh_claims.sid).await {
            Ok(_) => {}
            Err(e) => {
                log::warn!(
                    "ZChat refresh: session '{}' not active: {}",
                    refresh_claims.sid,
                    e
                );
                return HttpResponse::Unauthorized().json(ZchatErrorResponse {
                    error: "Session revoked or expired".to_string(),
                });
            }
        }
    }

    // 4) 签发新访问令牌：保留 sub/email/mfa_verified/sid，protocol="zchat"；
    //    iss/aud 由 encode_access_token 自动盖章。
    let mut claims = Claims::with_expiry_seconds(
        &refresh_claims.sub,
        &refresh_claims.email,
        refresh_claims.mfa_verified,
        state.jwt_access_expiry_secs,
    );
    claims.protocol = "zchat".to_string();
    claims.sid = refresh_claims.sid.clone();

    let access_token = match encode_access_token(&claims, &state.jwt_private_key) {
        Ok(t) => t,
        Err(e) => {
            log::error!("ZChat auth token encode error: {}", e);
            return HttpResponse::InternalServerError().json(ZchatErrorResponse {
                error: "Failed to generate token".to_string(),
            });
        }
    };

    HttpResponse::Ok().json(ZchatAuthResponse {
        access_token,
        token_type: "ZChat".to_string(),
        expires_in: state.jwt_access_expiry_secs.max(0) as u64,
    })
}

/// Configure routes (called from main.rs)
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth/zchat")
            .route("", web::post().to(zchat_auth))
            .route("/verify", web::post().to(zchat_verify)),
    );
}
