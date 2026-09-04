//! 登录/登出/刷新/MFA/me 核心 handler 与路由注册（自原 login.rs 纯拆分，零行为变化）。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use sqlx::{AssertSqlSafe, PgPool};

use super::tokens::{
    is_valid_refresh_token, record_failed_login, reset_failed_login, revoke_all_user_tokens,
    revoke_refresh_token, store_refresh_token,
};
use super::{
    get_client_ip, get_user_agent, list_sessions, revoke_other_sessions, revoke_session, AuthError,
    LoginRequest, LoginResponse, MfaLoginRequest, MfaLoginResponse, RefreshResponse,
};
use crate::auth::{
    crypto::decode_secret,
    jwt::{
        self, clear_access_cookie, clear_refresh_cookie, decode_token_any, encode_access_token,
        encode_refresh_token, set_access_cookie, set_refresh_cookie, Claims,
    },
    mfa::verify_totp_code,
    password::verify_password_async,
    session::{CreateSessionRequest, SessionManager},
    AuthState,
};
use crate::ngac::pdp::{evaluate_conditions, ConditionContext};

/// Login with email and password
pub async fn login(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    req: HttpRequest,
    body: web::Json<LoginRequest>,
) -> HttpResponse {
    // Auto-detect identifier type: email, username, or phone
    let identifier = &body.identifier;
    let (search_column, search_value) = if identifier.contains('@') {
        ("email", identifier)
    } else if identifier.starts_with('+')
        || (identifier.len() >= 8
            && identifier
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == ' ' || c == '(' || c == ')'))
    {
        ("phone", identifier)
    } else {
        ("username", identifier)
    };

    log::debug!("Login attempt with {}: {}", search_column, identifier);

    // Fetch user from database (incl. lockout counters — SECURITY_SPEC §5)
    // email 为可选认证链路（1:N 存于 auth_user_emails），登录经 auth_user_emails UNION
    // auth_users.email 解析；username/phone 仍按单列匹配。
    let where_expr = if search_column == "email" {
        "id IN (SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1 AND deleted_at IS NULL) \
         OR email = $1"
            .to_string()
    } else {
        format!("{} = $1", search_column)
    };
    let query = format!(
        "SELECT id, password_hash, COALESCE(mfa_enabled, false), mfa_secret, \
         COALESCE(failed_login_attempts, 0), locked_until \
         FROM isahl_auth.auth_users WHERE {}",
        where_expr
    );

    let user_result = sqlx::query_as::<
        _,
        (
            i64,
            Option<String>,
            bool,
            Option<String>,
            i32,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(AssertSqlSafe(query.as_str()))
    .bind(search_value)
    .fetch_optional(pool.get_ref())
    .await;

    let user = match user_result {
        Ok(Some(u)) => u,
        Ok(None) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid credentials".to_string(),
            })
        }
        Err(e) => {
            log::error!("Database error during login: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: format!("Login failed: {}", e),
            });
        }
    };

    let (user_id_i64, password_hash_opt, mfa_enabled, _mfa_secret, failed_attempts, locked_until) =
        user;

    // Account lockout check (SECURITY_SPEC §5): reject while locked, reporting remaining time.
    // Once `locked_until` has passed the lock is cleared and the failure counter reset, so the
    // user regains a full 5-attempt budget (auto-unlock-after-expiry).
    if let Some(locked) = locked_until {
        if locked > Utc::now() {
            let remaining = (locked - Utc::now()).num_minutes().max(0);
            return HttpResponse::Locked().json(AuthError {
                error: format!(
                    "ACCOUNT_LOCKED: account is temporarily locked, retry after {} minutes",
                    remaining
                ),
            });
        }
        // Lock expired: clear stale lock + counter before proceeding.
        reset_failed_login(pool.get_ref(), user_id_i64).await;
    }

    // Check if user has a password set
    let password_hash = match password_hash_opt {
        Some(h) => h,
        None => {
            log::warn!(
                "Login attempt for user {} without password_hash set",
                user_id_i64
            );
            // Count as a failed attempt (brute-force protection on orphaned accounts)
            record_failed_login(pool.get_ref(), user_id_i64, failed_attempts).await;
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid credentials".to_string(),
            });
        }
    };

    // User ID is already BIGINT

    // Verify password (offload CPU-intensive Argon2 to blocking pool)
    let new_hash = match verify_password_async(body.password.clone(), password_hash.clone()).await {
        Ok(Some(hash)) => hash,
        _ => {
            // Failed credential → increment counter, lock after threshold (SECURITY_SPEC §5)
            record_failed_login(pool.get_ref(), user_id_i64, failed_attempts).await;
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid credentials".to_string(),
            });
        }
    };

    // If hash was migrated, write the new standard hash back to DB
    if new_hash != password_hash {
        if let Err(e) = sqlx::query(
            "UPDATE isahl_auth.auth_users SET password_hash = $1, updated_at = NOW() WHERE id = $2",
        )
        .bind(&new_hash)
        .bind(user_id_i64)
        .execute(pool.get_ref())
        .await
        {
            log::warn!(
                "Password hash migration failed for user {}: {}",
                user_id_i64,
                e
            );
            // Non-fatal: login proceeds even if the update fails
        } else {
            log::info!(
                "Password hash migrated to standard argon2id for user {}",
                user_id_i64
            );
        }
    }

    // Successful credential verification — clear lockout state (SECURITY_SPEC §5)
    reset_failed_login(pool.get_ref(), user_id_i64).await;

    // Check user status before proceeding
    let user_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id_i64)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None);

    match user_status.as_deref() {
        Some("active") => {}
        Some("pending") => {
            return HttpResponse::Forbidden().json(AuthError {
                error: "IDENTITY_REQUIRED".to_string(),
            });
        }
        Some("identity_submitted") => {
            return HttpResponse::Forbidden().json(AuthError {
                error: "IDENTITY_UNDER_REVIEW".to_string(),
            });
        }
        Some("identity_verified") => {
            return HttpResponse::Forbidden().json(AuthError {
                error: "APPROVAL_PENDING".to_string(),
            });
        }
        Some("pending_approval") => {
            return HttpResponse::Forbidden().json(AuthError {
                error: "APPROVAL_PENDING".to_string(),
            });
        }
        Some("rejected") => {
            // refine-rejection-not-disabled：驳回 = 暂不授权——放行登录（前端展示驳回状态页，
            // 可手动再次发起审批申请）；无业务授权由「未挂 employee UA」天然保证。
        }
        _ => {
            return HttpResponse::Forbidden().json(AuthError {
                error: "ACCOUNT_INACTIVE".to_string(),
            });
        }
    }

    // If MFA is enabled, return mfa_required=true
    if mfa_enabled {
        // Create a pending session for MFA verification
        let session_manager = SessionManager::new(pool.get_ref().clone());
        let session = match session_manager
            .create_session(CreateSessionRequest {
                user_id: user_id_i64,
                ip_address: get_client_ip(&req),
                user_agent: get_user_agent(&req),
                ..Default::default()
            })
            .await
        {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to create session: {}", e);
                return HttpResponse::InternalServerError().json(AuthError {
                    error: "Failed to create session".to_string(),
                });
            }
        };

        return HttpResponse::Ok().json(LoginResponse {
            access_token: None,
            refresh_token: None,
            mfa_required: true,
            message: Some("MFA verification required".to_string()),
            session_id: Some(session.session_token),
        });
    }

    // No MFA - create session and issue tokens immediately
    let session_manager = SessionManager::new(pool.get_ref().clone());
    let session = match session_manager
        .create_session(CreateSessionRequest {
            user_id: user_id_i64,
            ip_address: get_client_ip(&req),
            user_agent: get_user_agent(&req),
            ..Default::default()
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create session: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to create session".to_string(),
            });
        }
    };

    // Generate tokens with session ID
    let mut claims = Claims::with_expiry_seconds(
        &user_id_i64.to_string(),
        "",
        false,
        state.jwt_access_expiry_secs,
    );
    // Bind token to the SSO session so the Gateway PEP can enforce
    // session revocation (logout) promptly.
    claims.sid = session.session_token.clone();

    let access_token = match encode_access_token(&claims, &state.jwt_private_key) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to generate token".to_string(),
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
                error: "Failed to generate refresh token".to_string(),
            })
        }
    };

    // Store refresh token in database
    use chrono::TimeDelta;
    let expires_at = Utc::now() + TimeDelta::seconds(state.jwt_refresh_expiry_secs);
    if let Err(e) =
        store_refresh_token(pool.get_ref(), user_id_i64, &refresh_token, expires_at).await
    {
        log::error!("Failed to store refresh token: {}", e);
    }

    let response = HttpResponse::Ok().json(LoginResponse {
        access_token: Some(access_token.clone()),
        refresh_token: Some(refresh_token.clone()),
        mfa_required: false,
        message: Some("Login successful".to_string()),
        session_id: Some(session.session_token.clone()),
    });

    let response = set_access_cookie(response, &access_token, state.jwt_access_expiry_secs);
    set_refresh_cookie(response, &refresh_token, state.jwt_refresh_expiry_secs)
}

/// Complete login with MFA code
pub async fn login_mfa(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    req: HttpRequest,
    body: web::Json<MfaLoginRequest>,
) -> HttpResponse {
    // Fetch user and MFA secret from database
    let user_result = sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT id, mfa_secret
        FROM isahl_auth.auth_users
        WHERE email = $1 AND mfa_enabled = true
        "#,
    )
    .bind(&body.email)
    .fetch_optional(pool.get_ref())
    .await;

    let (user_id, encrypted_secret) = match user_result {
        Ok(Some(u)) => u,
        Ok(None) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "User not found or MFA not enabled".to_string(),
            })
        }
        Err(e) => {
            log::error!("Database error during MFA login: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "MFA verification failed".to_string(),
            });
        }
    };

    // 还原 base32 字符串：支持 enc: 前缀密文（新）与旧明文 base32（迁移期兼容）。
    let base32_secret = match decode_secret(&state.encryption_key, &encrypted_secret) {
        Ok(s) => s,
        Err(e) => {
            log::error!("MFA login: failed to decode stored secret: {}", e);
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid MFA secret".to_string(),
            });
        }
    };

    // Decrypt MFA secret
    let secret_bytes =
        match base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &base32_secret) {
            Some(s) => s,
            None => {
                return HttpResponse::Unauthorized().json(AuthError {
                    error: "Invalid MFA secret".to_string(),
                })
            }
        };

    // Verify TOTP code
    if !verify_totp_code(&secret_bytes, &body.code) {
        return HttpResponse::Unauthorized().json(AuthError {
            error: "Invalid MFA code".to_string(),
        });
    }

    // MFA verified - issue tokens
    let claims = Claims::with_expiry_seconds(&user_id, "", true, state.jwt_access_expiry_secs); // mfa_verified = true

    let access_token = match encode_access_token(&claims, &state.jwt_private_key) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to generate token".to_string(),
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
                error: "Failed to generate refresh token".to_string(),
            })
        }
    };

    // 持久化 refresh token（与非 MFA 登录路径一致），
    // 否则 MFA 用户无法在 /auth/refresh 中校验、且登出时无法被 revoke-all 覆盖。
    use chrono::TimeDelta;
    let refresh_expires_at = Utc::now() + TimeDelta::seconds(state.jwt_refresh_expiry_secs);
    if let Ok(user_id_i64) = user_id.parse::<i64>() {
        if let Err(e) = store_refresh_token(
            pool.get_ref(),
            user_id_i64,
            &refresh_token,
            refresh_expires_at,
        )
        .await
        {
            log::error!("Failed to store MFA refresh token: {}", e);
        }
    } else {
        log::error!("Failed to parse MFA user_id '{}' as i64", user_id);
    }

    // Update or create session
    let session_manager = SessionManager::new(pool.get_ref().clone());

    // If session_id provided, validate it; otherwise create new session
    let session_token = if let Some(ref session_id) = body.session_id {
        // Validate existing session
        match session_manager.validate_session(session_id).await {
            Ok(session) => {
                if session.user_id.to_string() != user_id {
                    return HttpResponse::Unauthorized().json(AuthError {
                        error: "Session mismatch".to_string(),
                    });
                }
                session.session_token
            }
            Err(_) => {
                // Session invalid, create new one
                match session_manager
                    .create_session(CreateSessionRequest {
                        user_id: user_id.parse().unwrap(),
                        ip_address: get_client_ip(&req),
                        user_agent: get_user_agent(&req),
                        ..Default::default()
                    })
                    .await
                {
                    Ok(s) => s.session_token,
                    Err(e) => {
                        log::error!("Failed to create session: {}", e);
                        return HttpResponse::InternalServerError().json(AuthError {
                            error: "Failed to create session".to_string(),
                        });
                    }
                }
            }
        }
    } else {
        // Create new session
        match session_manager
            .create_session(CreateSessionRequest {
                user_id: user_id.parse().unwrap(),
                ip_address: get_client_ip(&req),
                user_agent: get_user_agent(&req),
                ..Default::default()
            })
            .await
        {
            Ok(s) => s.session_token,
            Err(e) => {
                log::error!("Failed to create session: {}", e);
                return HttpResponse::InternalServerError().json(AuthError {
                    error: "Failed to create session".to_string(),
                });
            }
        }
    };
    let response = HttpResponse::Ok().json(MfaLoginResponse {
        access_token: access_token.clone(),
        refresh_token: refresh_token.clone(),
        session_id: session_token,
    });

    let response = set_access_cookie(response, &access_token, state.jwt_access_expiry_secs);
    set_refresh_cookie(response, &refresh_token, state.jwt_refresh_expiry_secs)
}

/// Logout - invalidate session and tokens
pub async fn logout(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    // Try to get session token from header or cookie
    let session_token = req
        .headers()
        .get("X-Session-Token")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    // If we have a session token (from header), revoke it
    if let Some(ref token) = session_token {
        let session_manager = SessionManager::new(pool.get_ref().clone());
        if let Err(e) = session_manager.revoke_session(token, None, "logout").await {
            log::warn!("Failed to revoke session during logout: {}", e);
        }
    }

    // Fallback: extract session ID from access_token cookie
    if session_token.is_none() {
        if let Some(cookie) = req.cookie("access_token") {
            let access_token = cookie.value().to_string();
            if let Ok(claims) = decode_token_any(&access_token, &state.verification_keys()) {
                if !claims.sid.is_empty() {
                    let session_manager = SessionManager::new(pool.get_ref().clone());
                    if let Err(e) = session_manager
                        .revoke_session(&claims.sid, None, "logout")
                        .await
                    {
                        log::warn!("Failed to revoke session from access_token cookie: {}", e);
                    }
                }
            }
        }
    }

    // Also revoke refresh token from database
    if let Some(refresh_token) = req.cookie("refresh_token").map(|c| c.value().to_string()) {
        // Try to get user_id from token to revoke all tokens
        if let Ok(claims) = decode_token_any(&refresh_token, &state.verification_keys()) {
            let user_id = claims.sub.parse::<i64>().unwrap_or(0);
            if let Err(e) = revoke_all_user_tokens(pool.get_ref(), user_id).await {
                log::warn!("Failed to revoke refresh tokens: {}", e);
            }
        }
    }

    let response = clear_access_cookie(HttpResponse::Ok().finish());
    clear_refresh_cookie(response)
}

/// Refresh access token using refresh token
pub async fn refresh(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    // Extract refresh token from cookie first, fall back to Authorization header
    let refresh_token = match req.cookie("refresh_token") {
        Some(c) => c.value().to_string(),
        None => {
            // Fallback: Bearer token in Authorization header (for SPAs / cross-origin clients)
            match req
                .headers()
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
            {
                Some(token) => token.to_string(),
                None => {
                    return HttpResponse::Unauthorized().json(AuthError {
                        error: "No refresh token".to_string(),
                    })
                }
            }
        }
    };

    // Validate refresh token against database (check: exists, not revoked, not expired)
    match is_valid_refresh_token(pool.get_ref(), &refresh_token).await {
        Ok(true) => {}
        Ok(false) => {
            log::warn!("Invalid or expired refresh token");
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or expired refresh token".to_string(),
            });
        }
        Err(e) => {
            log::error!("Database error during refresh token validation: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Token validation failed".to_string(),
            });
        }
    }

    // Decode and validate refresh token JWT
    let claims = match decode_token_any(&refresh_token, &state.verification_keys()) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Refresh token decode error: {}", e);
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid refresh token".to_string(),
            });
        }
    };

    // fix-approval-endpoint-gates：refresh 状态门禁（与 login 门禁同语义）——
    // disabled/pending_approval/pending 用户不得刷新 token（登录绕过漏洞）；
    // rejected 放行（维持「可登录重新申请」登录态）。
    let gate_user_id: i64 = claims.sub.parse::<i64>().unwrap_or(0);
    let gate_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
            .bind(gate_user_id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None);
    match gate_status.as_deref() {
        Some("active") | Some("rejected") => {}
        Some("pending_approval") => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "APPROVAL_PENDING".to_string(),
            });
        }
        _ => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "ACCOUNT_INACTIVE".to_string(),
            });
        }
    }

    // Revoke old refresh token (rotation)
    if let Err(e) = revoke_refresh_token(pool.get_ref(), &refresh_token).await {
        log::error!("Failed to revoke old refresh token: {}", e);
    }

    // Issue new access token using refresh_access_token helper
    let (new_access_token, _new_claims) = match jwt::refresh_access_token(
        &claims,
        &state.jwt_private_key,
        state.jwt_access_expiry_secs,
    ) {
        Ok(result) => result,
        Err(e) => {
            log::error!("Failed to refresh access token: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to generate token".to_string(),
            });
        }
    };

    // Issue new refresh token (rotation)
    let new_refresh_token = match encode_refresh_token(
        &claims,
        &state.jwt_private_key,
        state.jwt_refresh_expiry_secs,
    ) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to generate refresh token".to_string(),
            })
        }
    };

    // Store new refresh token in database
    use chrono::TimeDelta;
    let expires_at = Utc::now() + TimeDelta::seconds(state.jwt_refresh_expiry_secs);
    let user_id_i64 = claims.sub.parse::<i64>().unwrap_or(0);
    if let Err(e) =
        store_refresh_token(pool.get_ref(), user_id_i64, &new_refresh_token, expires_at).await
    {
        log::error!("Failed to store new refresh token: {}", e);
    }
    let response = HttpResponse::Ok().json(RefreshResponse {
        access_token: new_access_token.clone(),
        refresh_token: new_refresh_token.clone(),
        token_type: "Bearer".to_string(),
        expires_in: state.jwt_access_expiry_secs.max(0) as u64,
    });

    let response = set_access_cookie(response, &new_access_token, state.jwt_access_expiry_secs);
    set_refresh_cookie(response, &new_refresh_token, state.jwt_refresh_expiry_secs)
}

/// Current user info (GET /auth/me)
///
/// Returns the authenticated user's profile data.
/// Requires valid JWT access_token in Cookie or Authorization header.
pub async fn me(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    // Extract and validate access token
    let access_token = match req.cookie("access_token") {
        Some(c) => c.value().to_string(),
        None => {
            // Try Authorization header fallback
            match req
                .headers()
                .get(actix_web::http::header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|auth| auth.strip_prefix("Bearer "))
            {
                Some(token) => token.to_string(),
                None => {
                    return HttpResponse::Unauthorized().json(AuthError {
                        error: "No authentication token".to_string(),
                    })
                }
            }
        }
    };

    // Decode and validate JWT
    let claims = match decode_token_any(&access_token, &state.verification_keys()) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Access token decode error in /auth/me: {}", e);
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or expired token".to_string(),
            });
        }
    };

    // Parse user ID from claims
    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid user ID in token".to_string(),
            })
        }
    };

    // Look up user from database with NGAC attributes and accessible modules
    let user_row = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            bool,
            String,
            Vec<String>,
            Vec<String>,
        ),
    >(
        r#"
        SELECT
            u.id::TEXT,
            u.username,
            u.email,
            u.name,
            u.display_name,
            u.user_type,
            u.is_active,
            u.status,
            COALESCE(
                array_agg(DISTINCT ua.o_name) FILTER (WHERE ua.o_name IS NOT NULL),
                '{}'::TEXT[]
            ) AS ngac_user_attributes,
            COALESCE(
                array_agg(DISTINCT oa.resource_identifier) FILTER (
                    WHERE oa.resource_type = 'module' AND oa.resource_identifier IS NOT NULL
                ),
                '{}'::TEXT[]
            ) AS accessible_modules
        FROM isahl_auth.auth_users u
        LEFT JOIN isahl_auth.ngac_user_rr_attribute ur
            ON ur.fk_user = u.id
            AND ur.deleted_at IS NULL
            AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
        LEFT JOIN isahl_auth.ngac_user_attribute ua
            ON ua.id = ur.fk_user_attribute
        LEFT JOIN isahl_auth.ngac_association assoc
            ON assoc.fk_user_attribute = ua.id
        LEFT JOIN isahl_auth.ngac_object_attribute oa
            ON oa.id = assoc.fk_object_attribute
        WHERE u.id = $1 AND u.is_active = true
        GROUP BY u.id, u.username, u.email, u.name, u.display_name, u.user_type, u.is_active, u.status
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await;

    match user_row {
        Ok(Some((
            id,
            username,
            email,
            name,
            display_name,
            user_type,
            is_active,
            status,
            ngac_attrs,
            modules,
        ))) => {
            // 从 NGAC 属性推导 portal-scope
            let has_workbench = ngac_attrs.iter().any(|a| a == "admin" || a == "operator");
            let has_storefront = ngac_attrs
                .iter()
                .any(|a| a == "user" || a == "customer" || a == "storefront");
            let mut portal_scope: Vec<&str> = Vec::new();
            if has_workbench {
                portal_scope.push("workbench");
            }
            if has_storefront {
                portal_scope.push("storefront");
            }
            // 无 NGAC 属性或未推导出 scope 时默认 workbench
            if portal_scope.is_empty() {
                portal_scope.push("workbench");
            }
            let portal_default = if has_storefront && !has_workbench {
                "storefront"
            } else {
                "workbench"
            };

            // ── NGAC 逐资源权限矩阵（与 PDP 同一关联集；条件按 NGAC_SPEC §2.4 fail-closed 求值）──
            // 供 Gateway 前端 usePermission/PermissionGate 做 UI 显隐判定；服务端 PEP/PDP 仍为准。
            // v2 条件上下文（add-ngac-condition-v2）：用户有效 UA 名 = ngac_attrs（直接指派）
            // ∪ 认知/委托派生名（effective_ua 同源）；OA 闭包名 = 集合级 OA 名全集
            // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）——
            // 派生名查询引用 ngac_delegation（DELEGATED_CTE），MUST 先行
            crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
            let derived_ua_names: Vec<String> = {
                let derived_sql = format!(
                    r#"
                    WITH {COGNITION_CTE},
                    {DELEGATED_CTE},
                    effective_ua AS (
                        SELECT ur.fk_user_attribute AS ua_id
                        FROM isahl_auth.ngac_user_rr_attribute ur
                        WHERE ur.fk_user = $1
                          AND ur.deleted_at IS NULL
                          AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
                        UNION
                        SELECT ua2.id
                        FROM isahl_auth.ngac_user_attribute ua2
                        JOIN cognition_ua_names cn ON cn.o_name = ua2.o_name
                        WHERE ua2.deleted_at IS NULL
                        UNION
                        SELECT ua3.id
                        FROM delegated_ua du
                        JOIN isahl_auth.ngac_user_attribute ua3 ON ua3.id = du.id
                        WHERE ua3.deleted_at IS NULL
                    )
                    SELECT ua.o_name FROM isahl_auth.ngac_user_attribute ua
                    JOIN effective_ua eu ON eu.ua_id = ua.id
                    WHERE ua.deleted_at IS NULL
                    "#,
                    COGNITION_CTE = common::ngac_org::COGNITION_CTE,
                    DELEGATED_CTE = common::ngac_org::DELEGATED_CTE,
                );
                let names: Vec<String> =
                    sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(derived_sql.as_str()))
                        .bind(user_id)
                        .fetch_all(pool.get_ref())
                        .await
                        .unwrap_or_default();
                names
            };
            let mut user_ua_names: Vec<String> = ngac_attrs.clone();
            for n in derived_ua_names {
                if !user_ua_names.contains(&n) {
                    user_ua_names.push(n);
                }
            }
            let collection_oa_names: Vec<String> = sqlx::query_scalar(
                "SELECT o_name FROM isahl_auth.ngac_object_attribute \
                 WHERE deleted_at IS NULL AND fk_resource = 0",
            )
            .fetch_all(pool.get_ref())
            .await
            .unwrap_or_default();
            let ctx = ConditionContext {
                now: Utc::now(),
                user_ua_names,
                oa_closure_names: collection_oa_names,
            };
            // 认知派生 UA 先物化（幂等，warn 降级），再并入有效 UA 集
            // （add-ngac-cognition-derived-ua D3：与 PIP 同一 CTE 常量，不含祖先闭包的既有矩阵语义不变）
            common::ngac_org::ensure_cognition_uas(pool.get_ref(), user_id).await;
            let perm_sql = format!(
                r#"
                WITH {COGNITION_CTE},
                {DELEGATED_CTE},
                effective_ua AS (
                    SELECT ur.fk_user_attribute AS ua_id
                    FROM isahl_auth.ngac_user_rr_attribute ur
                    WHERE ur.fk_user = $1
                      AND ur.deleted_at IS NULL
                      AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
                    UNION
                    SELECT ua2.id
                    FROM isahl_auth.ngac_user_attribute ua2
                    JOIN cognition_ua_names cn ON cn.o_name = ua2.o_name
                    WHERE ua2.deleted_at IS NULL
                    UNION
                    -- 委托派生 UA（add-ngac-delegation D2，与 PIP 同 CTE 同构）
                    SELECT ua3.id
                    FROM delegated_ua du
                    JOIN isahl_auth.ngac_user_attribute ua3 ON ua3.id = du.id
                    WHERE ua3.deleted_at IS NULL
                )
                SELECT oa.resource_type, ar.o_name, assoc.conditions
                FROM effective_ua eu
                JOIN isahl_auth.ngac_user_attribute ua ON ua.id = eu.ua_id
                JOIN isahl_auth.ngac_association assoc ON assoc.fk_user_attribute = ua.id
                JOIN isahl_auth.ngac_object_attribute oa ON oa.id = assoc.fk_object_attribute
                JOIN isahl_auth.ngac_access_right ar ON ar.id = ANY(assoc.ak_access_rights)
                WHERE ua.deleted_at IS NULL
                  AND assoc.deleted_at IS NULL
                  AND oa.deleted_at IS NULL
                "#,
                COGNITION_CTE = common::ngac_org::COGNITION_CTE,
                DELEGATED_CTE = common::ngac_org::DELEGATED_CTE
            );
            // SQL 由编译期常量 common::ngac_org::COGNITION_CTE 拼装（无用户输入），AssertSqlSafe 显式审计
            let perm_rows = sqlx::query_as::<_, (String, String, Option<serde_json::Value>)>(
                sqlx::AssertSqlSafe(perm_sql.as_str()),
            )
            .bind(user_id)
            .fetch_all(pool.get_ref())
            .await;

            let mut perm_map: std::collections::HashMap<
                String,
                std::collections::BTreeSet<String>,
            > = std::collections::HashMap::new();
            match perm_rows {
                Ok(rows) => {
                    for (resource_type, action, conditions) in rows {
                        if !evaluate_conditions(&conditions, &ctx) {
                            // 条件不满足（含非法字段，失败封闭）→ 该条关联不授予
                            continue;
                        }
                        perm_map.entry(resource_type).or_default().insert(action);
                    }
                }
                Err(e) => {
                    // 权限矩阵失败不阻断 me：降级为空矩阵（安全侧收紧），仅告警
                    log::warn!(
                        "Failed to resolve NGAC permissions for user {}: {}",
                        user_id,
                        e
                    );
                }
            }
            // prohibition 扣减（deny-overrides 对齐，fix-ngac-decision-consistency D6）：
            // 用户有效 UA 集命中的 active prohibition（conditions 同一 fail-closed 求值）
            // 对应 (resource_type, action) 从矩阵剔除——UI 显隐与服务端裁决一致。
            let denial_sql = format!(
                r#"
                WITH {COGNITION_CTE},
                {DELEGATED_CTE},
                effective_ua AS (
                    SELECT ur.fk_user_attribute AS ua_id
                    FROM isahl_auth.ngac_user_rr_attribute ur
                    WHERE ur.fk_user = $1
                      AND ur.deleted_at IS NULL
                      AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
                    UNION
                    SELECT ua2.id
                    FROM isahl_auth.ngac_user_attribute ua2
                    JOIN cognition_ua_names cn ON cn.o_name = ua2.o_name
                    WHERE ua2.deleted_at IS NULL
                    UNION
                    SELECT ua3.id
                    FROM delegated_ua du
                    JOIN isahl_auth.ngac_user_attribute ua3 ON ua3.id = du.id
                    WHERE ua3.deleted_at IS NULL
                )
                SELECT oa.resource_type, ar.o_name, p.conditions
                FROM effective_ua eu
                JOIN isahl_auth.ngac_prohibition p ON p.fk_user_attribute = eu.ua_id
                JOIN isahl_auth.ngac_object_attribute oa ON oa.id = p.fk_object_attribute
                JOIN isahl_auth.ngac_access_right ar ON ar.id = ANY(p.ak_access_rights)
                WHERE p.is_active AND p.deleted_at IS NULL AND oa.deleted_at IS NULL
                "#,
                COGNITION_CTE = common::ngac_org::COGNITION_CTE,
                DELEGATED_CTE = common::ngac_org::DELEGATED_CTE
            );
            match sqlx::query_as::<_, (String, String, Option<serde_json::Value>)>(
                sqlx::AssertSqlSafe(denial_sql.as_str()),
            )
            .bind(user_id)
            .fetch_all(pool.get_ref())
            .await
            {
                Ok(rows) => {
                    for (resource_type, action, conditions) in rows {
                        if !evaluate_conditions(&conditions, &ctx) {
                            // 条件不满足（失败封闭）→ 该条 prohibition 不生效，不扣减
                            continue;
                        }
                        if let Some(actions) = perm_map.get_mut(&resource_type) {
                            actions.remove(&action);
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "Failed to resolve NGAC prohibition subtraction for user {}: {}",
                        user_id,
                        e
                    );
                }
            }
            let permissions: serde_json::Map<String, serde_json::Value> = perm_map
                .iter()
                .map(|(rt, actions)| {
                    let list: Vec<&str> = actions.iter().map(|a| a.as_str()).collect();
                    (rt.clone(), serde_json::json!(list))
                })
                .collect();

            // ── 岗位解析（isahl 登录可查自身所在岗位）──
            // 链：auth_users.id → (zc_id_empl-agent / zc_id_empl-natural 的 fk_user)
            //      → zc_id_subj-post_rr_employee(ref_right=雇员) → zc_id_subj-position(ref_left=岗位)
            // 与 Framework/backend/contacts 岗位解析链同构；失败降级为空数组 + 告警（不阻断 me）。
            let position_rows: Vec<(i64, String, Option<String>)> =
                match sqlx::query_as::<_, (i64, String, Option<String>)>(
                    r#"
                SELECT sp.id, sp.code, sp.notice
                FROM isahl."zc_id_empl-agent" ea
                JOIN isahl."zc_id_subj-post_rr_employee" spre
                    ON spre.ref_right = ea.id AND spre.deleted_at IS NULL
                JOIN isahl."zc_id_subj-position" sp
                    ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
                WHERE ea.fk_user = $1 AND ea.deleted_at IS NULL
                UNION
                SELECT sp.id, sp.code, sp.notice
                FROM isahl."zc_id_empl-natural" en
                JOIN isahl."zc_id_subj-post_rr_employee" spre
                    ON spre.ref_right = en.id AND spre.deleted_at IS NULL
                JOIN isahl."zc_id_subj-position" sp
                    ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
                WHERE en.fk_user = $1 AND en.deleted_at IS NULL
                "#,
                )
                .bind(user_id)
                .fetch_all(pool.get_ref())
                .await
                {
                    Ok(rows) => rows,
                    Err(e) => {
                        log::warn!("Failed to resolve positions for user {}: {}", user_id, e);
                        Vec::new()
                    }
                };
            let positions: Vec<serde_json::Value> = position_rows
                .iter()
                .map(|(id, code, notice)| {
                    serde_json::json!({
                        "id": id.to_string(),
                        "code": code,
                        "name": notice.clone().unwrap_or_default(),
                    })
                })
                .collect();

            // ── 主体认知（refactor-subject-perspective-chain）──
            // auth_users.entity_table/entity_id → zc_id_subjects 继承链父表统一解析
            // （empl-natural / orga-non-banking-legal 等全部可绑定实体均为 subjects 后代，
            // 父表查询继承链统一可见，无需按 entity_table 分派）。
            // 未绑定/悬空/失败 → null + 告警（不阻断 me）。
            let subject: serde_json::Value =
                match sqlx::query_as::<_, (Option<String>, Option<i64>)>(
                    "SELECT entity_table, entity_id FROM isahl_auth.auth_users WHERE id = $1",
                )
                .bind(user_id)
                .fetch_optional(pool.get_ref())
                .await
                {
                    Ok(Some((Some(table), Some(entity_id)))) if entity_id > 0 => {
                        match sqlx::query_as::<_, (Option<String>, Option<String>)>(
                            "SELECT code, notice FROM isahl.zc_id_subjects \
                         WHERE id = $1 AND deleted_at IS NULL",
                        )
                        .bind(entity_id)
                        .fetch_optional(pool.get_ref())
                        .await
                        {
                            Ok(Some((code, notice))) => serde_json::json!({
                                // zuid 量级超 JS 安全整数——字符串化（对齐 common::serde_zuid 约定）
                                "id": entity_id.to_string(),
                                "code": code.unwrap_or_default(),
                                "name": notice.unwrap_or_default(),
                                "entity_table": table,
                            }),
                            Ok(None) => {
                                log::warn!(
                                    "User {} 主体绑定悬空: entity_table={} entity_id={}",
                                    user_id,
                                    table,
                                    entity_id
                                );
                                serde_json::Value::Null
                            }
                            Err(e) => {
                                log::warn!("Failed to resolve subject for user {}: {}", user_id, e);
                                serde_json::Value::Null
                            }
                        }
                    }
                    Ok(_) => serde_json::Value::Null,
                    Err(e) => {
                        log::warn!("Failed to read entity binding for user {}: {}", user_id, e);
                        serde_json::Value::Null
                    }
                };

            // ── 视角解析（主体→岗位→视角标签链的「我」侧）──
            // 视角标签为岗位级属性（relation-post_view_r_tags: ref_left=岗位, ref_right=标签）：
            // 我的视角 = 我的岗位集合各自的标签集合。失败降级为空数组 + 告警（不阻断 me）。
            let perspectives: Vec<serde_json::Value> = if position_rows.is_empty() {
                Vec::new()
            } else {
                let position_ids: Vec<i64> = position_rows.iter().map(|(id, _, _)| *id).collect();
                match sqlx::query_as::<_, (i64, String, Option<String>)>(
                    r#"SELECT r.ref_left, vt.code, vt.notice
                       FROM isahl."zc_id_relation-post_view_r_tags" r
                       JOIN isahl."zc_id_tags-post_view" vt
                           ON vt.id = r.ref_right AND vt.deleted_at IS NULL
                       WHERE r.ref_left = ANY($1) AND r.deleted_at IS NULL
                       ORDER BY r.ref_left, vt.o_number, vt.id"#,
                )
                .bind(&position_ids)
                .fetch_all(pool.get_ref())
                .await
                {
                    Ok(tag_rows) => position_rows
                        .iter()
                        .map(|(pid, pcode, pnotice)| {
                            let view_tags: Vec<serde_json::Value> = tag_rows
                                .iter()
                                .filter(|(ref_left, _, _)| ref_left == pid)
                                .map(|(_, tcode, tnotice)| {
                                    serde_json::json!({
                                        "code": tcode,
                                        "name": tnotice.clone().unwrap_or_default(),
                                    })
                                })
                                .collect();
                            serde_json::json!({
                                "position_id": pid.to_string(),
                                "position_code": pcode,
                                "position_name": pnotice.clone().unwrap_or_default(),
                                "view_tags": view_tags,
                            })
                        })
                        .collect(),
                    Err(e) => {
                        log::warn!("Failed to resolve perspectives for user {}: {}", user_id, e);
                        Vec::new()
                    }
                }
            };

            // ── 邮箱列表（1:N，auth_user_emails；email 为可选认证链路，非唯一基点）──
            let emails: Vec<serde_json::Value> =
                match sqlx::query_as::<_, (String, bool)>(
                    "SELECT email, is_primary FROM isahl_auth.auth_user_emails \
                     WHERE fk_user = $1 AND deleted_at IS NULL ORDER BY is_primary DESC, id ASC",
                )
                .bind(user_id)
                .fetch_all(pool.get_ref())
                .await
                {
                    Ok(rows) => rows
                        .into_iter()
                        .map(|(email, is_primary)| {
                            serde_json::json!({ "email": email, "is_primary": is_primary })
                        })
                        .collect(),
                    Err(e) => {
                        log::warn!("Failed to resolve emails for user {}: {}", user_id, e);
                        Vec::new()
                    }
                };

            HttpResponse::Ok().json(serde_json::json!({
                "user": {
                    "id": id.to_string(),
                    "user_type": user_type,
                    "email": email,
                    "name": name.unwrap_or_default(),
                    "display_name": display_name.or(username.clone()),
                    "is_active": is_active,
                    "status": status,
                    "ngac_user_attributes": ngac_attrs,
                    "attributes": {
                        "portal-scope": portal_scope,
                        "portal-default": portal_default,
                    },
                    "accessible_modules": modules,
                    "permissions": permissions,
                    "positions": positions,
                    "subject": subject,
                    "perspectives": perspectives,
                    "emails": emails,
                }
            }))
        }
        Ok(None) => HttpResponse::NotFound().json(AuthError {
            error: "User not found".to_string(),
        }),
        Err(e) => {
            log::error!("Database error in /auth/me: {}", e);
            HttpResponse::InternalServerError().json(AuthError {
                error: "Internal server error".to_string(),
            })
        }
    }
}

/// Configure auth routes
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/login", web::post().to(login))
        .route("/login/mfa", web::post().to(login_mfa))
        .route("/login/ldap", web::post().to(crate::auth::ldap::ldap_login))
        .route("/register", web::post().to(crate::auth::register::register))
        // 外部主体注册通道（add-dual-register-channels）：OpenActivity 门户专用
        .route(
            "/register/external",
            web::post().to(crate::auth::register::register_external),
        )
        .route("/me", web::get().to(me))
        .route(
            "/ldap/configs",
            web::get().to(crate::auth::ldap::list_ldap_configs),
        )
        .route(
            "/ldap/test",
            web::post().to(crate::auth::ldap::test_ldap_connection),
        )
        .route("/logout", web::post().to(logout))
        .route("/refresh", web::post().to(refresh))
        .route("/sessions", web::get().to(list_sessions))
        // 注意：/sessions/all 必须先于 /sessions/{token} 注册，否则 "all" 被 {token} 捕获
        .route("/sessions/all", web::delete().to(revoke_other_sessions))
        .route("/sessions/{token}", web::delete().to(revoke_session))
        .route(
            "/portal-context",
            web::get().to(crate::auth::portal::get_portal_context),
        )
        .route(
            "/check-access",
            web::post().to(crate::auth::check_access::check_access),
        )
        .configure(crate::auth::email::configure_routes)
        .configure(crate::auth::sms::configure_routes);
}
