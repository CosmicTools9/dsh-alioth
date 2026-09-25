use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{TimeDelta, Utc};
use rand::RngExt;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::auth::login::AuthError;

/// POST /auth/reset-password/request
/// 接收 email，生成 token，hash 后存入 DB，记录日志供管理员调试
pub async fn request_reset(
    pool: web::Data<PgPool>,
    email_service: web::Data<Box<dyn common::EmailService>>,
    body: web::Json<ResetRequest>,
) -> HttpResponse {
    let email = &body.email;

    // 候选账号（allow-duplicate-email-accounts）：同一 email MAY 属于多个 active 账号
    // ⇒ 为每个匹配账号各生成独立 token 并各投一封（邮箱主人对其名下全部账号可恢复）；
    // 单请求投递上限 5 封（超出不投递并 warn）；对外恒返回同一成功文案（防枚举）。
    let mut user_ids: Vec<i64> = match crate::auth::identifier::resolve_candidate_ids(
        pool.get_ref(),
        crate::auth::identifier::IdentifierColumn::Email,
        email,
    )
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            log::error!("Password reset: candidate resolution failed: {}", e);
            Vec::new()
        }
    };
    if !user_ids.is_empty() {
        user_ids = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM isahl_auth.auth_users \
             WHERE id = ANY($1) AND is_active = true ORDER BY id",
        )
        .bind(&user_ids)
        .fetch_all(pool.get_ref())
        .await
        .unwrap_or_default();
    }

    const MAX_RESET_ACCOUNTS_PER_REQUEST: usize = 5;
    if user_ids.len() > MAX_RESET_ACCOUNTS_PER_REQUEST {
        log::warn!(
            "Password reset: email 命中 {} 个 active 账号，超过单请求上限 {}，超出部分不投递",
            user_ids.len(),
            MAX_RESET_ACCOUNTS_PER_REQUEST
        );
        user_ids.truncate(MAX_RESET_ACCOUNTS_PER_REQUEST);
    }

    // Cleanup expired tokens (no periodic job exists, do it opportunistically)
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.password_reset_tokens WHERE expires_at < NOW() - INTERVAL '1 day'",
    )
    .execute(pool.get_ref())
    .await;

    for user_id in user_ids {
        // 生成随机 token
        let token: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        let token_hash = hex::encode(Sha256::digest(token.as_bytes()));

        // 失效该账号所有未使用的 token
        let _ = sqlx::query(
            "UPDATE isahl_auth.password_reset_tokens SET used_at = NOW() WHERE user_id = $1 AND used_at IS NULL",
        )
        .bind(user_id)
        .execute(pool.get_ref())
        .await;

        // 存入新 token（10 分钟有效）
        let expires_at = Utc::now() + TimeDelta::minutes(10);
        let _ = sqlx::query(
            "INSERT INTO isahl_auth.password_reset_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(&token_hash)
        .bind(expires_at)
        .execute(pool.get_ref())
        .await;

        // R2: Always log the token (admins can find it in server logs during debugging),
        // never return it in the response body — avoids leaking reset tokens in production.
        log::info!(
            "Password reset token generated for user {}: {}",
            user_id,
            token
        );

        // 发送重置邮件（收件地址 = 请求 email；该 email 可能对应多个账号，各账号独立 token）
        let subject = "密码重置";
        let reset_link = format!(
            "{}/auth/reset-password?token={}",
            super::oauth_callback::get_base_url(),
            token
        );
        let body_text = format!(
            "您的密码重置链接（10分钟内有效）：\n\n{}\n\n如果非您本人操作，请忽略此邮件。",
            reset_link
        );
        if let Err(e) = email_service.send(email, subject, &body_text).await {
            log::error!("Failed to send password reset email to {}: {}", email, e);
        } else {
            log::info!("Password reset email sent to {}", email);
        }
    }

    HttpResponse::Ok().json(serde_json::json!({
        "message": "If the email exists, a reset code has been sent"
    }))
}

/// POST /auth/reset-password/confirm
/// 接收 token + new_password，验证后重置密码并撤销所有 session
pub async fn confirm_reset(
    _req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ResetConfirm>,
) -> HttpResponse {
    let token_hash = hex::encode(Sha256::digest(body.token.as_bytes()));

    // 查找有效 token
    let token_row: Option<(i64, i64)> = sqlx::query_as(
        r#"
        SELECT id, user_id FROM isahl_auth.password_reset_tokens
        WHERE token_hash = $1 AND used_at IS NULL AND expires_at > NOW()
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(&token_hash)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let Some((token_id, user_id)) = token_row else {
        return HttpResponse::BadRequest().json(AuthError {
            error: "Invalid or expired reset token".to_string(),
        });
    };

    // 哈希新密码
    let password_hash =
        match crate::auth::password::hash_password_async(body.new_password.clone()).await {
            Ok(h) => h,
            Err(e) => {
                log::error!("Password hashing failed: {}", e);
                return HttpResponse::InternalServerError().json(AuthError {
                    error: "Password processing failed".to_string(),
                });
            }
        };

    // 更新密码
    let _ = sqlx::query(
        "UPDATE isahl_auth.auth_users SET password_hash = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(&password_hash)
    .bind(user_id)
    .execute(pool.get_ref())
    .await;

    // 标记 token 已使用
    let _ =
        sqlx::query("UPDATE isahl_auth.password_reset_tokens SET used_at = NOW() WHERE id = $1")
            .bind(token_id)
            .execute(pool.get_ref())
            .await;

    // 撤销该用户所有 session（强制重新登录）
    let _ = sqlx::query(
        "UPDATE isahl_auth.sso_sessions SET status = 'revoked', updated_at = NOW() WHERE user_id = $1 AND status = 'active'"
    )
    .bind(user_id)
    .execute(pool.get_ref())
    .await;

    // R9: Clear httpOnly cookies so the user's browser doesn't retain stale JWTs
    let response = HttpResponse::Ok().json(serde_json::json!({
        "message": "Password reset successful. Please log in again."
    }));
    let response = crate::auth::jwt::clear_access_cookie(response, None);

    crate::auth::jwt::clear_refresh_cookie(response, None)
}

#[derive(serde::Deserialize)]
pub struct ResetRequest {
    pub email: String,
}

#[derive(serde::Deserialize)]
pub struct ResetConfirm {
    pub token: String,
    pub new_password: String,
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    // 相对 scope（/reset-password）：挂载方决定前缀——scope("/auth") 内 = /auth/reset-password，
    // Gateway 反代 scope("/api/auth") 内 = /api/auth/reset-password（与 login "/login" 同一约定；
    // 此前绝对 "/auth/reset-password" 在两种挂法下都双重前缀，端点实际不可达）
    cfg.service(
        web::scope("/reset-password")
            .route("/request", web::post().to(request_reset))
            .route("/confirm", web::post().to(confirm_reset)),
    );
}
