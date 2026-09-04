//! 用户修改密码 handler
//!
//! 用户已登录状态下修改自己的密码。
//! 验证旧密码 → 写入新密码 → 撤销其他 session。
//!
//! POST /auth/password/change
//! Authorization: Bearer <token> 或 Cookie: access_token=<token>

use actix_web::{web, HttpRequest, HttpResponse};
use serde::Deserialize;

use super::jwt::{decode_token_any, Claims};
use super::login::AuthError;
use super::password::{self, hash_password_async, verify_password_async};
use super::session::SessionManager;

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// POST /auth/password/change
///
/// 验证旧密码 → 写入新密码 → 撤销当前用户所有其他 session。
pub async fn change_password(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<super::AuthState>,
    body: web::Json<ChangePasswordRequest>,
) -> HttpResponse {
    // 1. 验证 token 提取用户
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
                return HttpResponse::Unauthorized().json(AuthError {
                    error: "No authentication token".to_string(),
                })
            }
        },
    };

    let claims: Claims = match decode_token_any(&access_token, &state.verification_keys()) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Token decode error in password/change: {}", e);
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or expired token".to_string(),
            });
        }
    };

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid user ID in token".to_string(),
            })
        }
    };

    // 2. 验证新密码策略（集中校验，SECURITY_SPEC §5 基线）
    if let Err(e) = password::validate_password_policy(&body.new_password) {
        return HttpResponse::BadRequest().json(AuthError {
            error: e.to_string(),
        });
    }

    // 3. 查询当前密码哈希
    let password_hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None)
            .flatten();

    let current_hash = match password_hash {
        Some(h) => h,
        None => {
            return HttpResponse::BadRequest().json(AuthError {
                error: "No password set for this account".to_string(),
            })
        }
    };

    // 4. 验证旧密码
    match verify_password_async(body.old_password.clone(), current_hash).await {
        Ok(Some(_)) => {} // password matches, hash may have been migrated
        Ok(None) => {
            return HttpResponse::Forbidden().json(AuthError {
                error: "Old password is incorrect".to_string(),
            })
        }
        Err(e) => {
            log::error!("Password verification error: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to verify password".to_string(),
            });
        }
    }

    // 5. 新密码不能与旧密码相同
    if body.old_password == body.new_password {
        return HttpResponse::BadRequest().json(AuthError {
            error: "New password must be different from old password".to_string(),
        });
    }

    // 6. 哈希新密码
    let new_hash = match hash_password_async(body.new_password.clone()).await {
        Ok(h) => h,
        Err(e) => {
            log::error!("Failed to hash new password: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to process new password".to_string(),
            });
        }
    };

    // 7. 更新密码
    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.auth_users SET password_hash = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(&new_hash)
    .bind(user_id)
    .execute(pool.get_ref())
    .await
    {
        log::error!("Failed to update password for user {}: {}", user_id, e);
        return HttpResponse::InternalServerError().json(AuthError {
            error: "Failed to update password".to_string(),
        });
    }

    // 8. 撤销该用户所有 refresh token（防止续期）
    let _ = sqlx::query("UPDATE isahl_auth.refresh_tokens SET revoked = TRUE WHERE user_id = $1")
        .bind(user_id)
        .execute(pool.get_ref())
        .await;

    // 9. 吊销该用户所有活跃 sso_sessions，使已签发的 access_token 在会话层失效
    //    （access_token 本身为无状态 JWT，仍可在 15 分钟窗口内使用，但无法据此建立/恢复会话）
    let session_manager = SessionManager::new(pool.get_ref().clone());
    if let Err(e) = session_manager
        .revoke_all_user_sessions(user_id, None, Some(user_id), "password_change")
        .await
    {
        log::error!(
            "Failed to revoke sessions after password change for user {}: {}",
            user_id,
            e
        );
    }

    log::info!("Password changed for user {}", user_id);

    HttpResponse::Ok().json(serde_json::json!({
        "message": "Password changed successfully. Other sessions have been revoked."
    }))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/auth/password/change", web::post().to(change_password));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::session::is_session_active;

    /// 连接规范测试库（与 scripts/db/reset-db.sh --test 约定一致）。
    /// 可通过 `DATABASE_URL` 或 `SSO_TEST_DATABASE_URL` 覆盖。
    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
            .unwrap_or_else(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "william.d.zk".to_string());
                format!("postgres://{}@localhost:5432/aliothstudio_test", user)
            });
        sqlx::PgPool::connect(&url)
            .await
            .expect("无法连接测试库，请先运行 `bash scripts/db/reset-db.sh --test`")
    }

    /// 改密后应吊销该用户全部活跃 session，使旧 token 在会话层失效。
    ///
    /// 直接复用 password_change.rs 新增的 `revoke_all_user_sessions` 调用逻辑，
    /// 验证 `is_session_active` 对已被吊销 session 的 token 返回 false。
    #[tokio::test]
    async fn password_change_revokes_active_sessions() {
        let pool = test_pool().await;
        let suffix = uuid::Uuid::new_v4().to_string();

        let user_id: i64 = sqlx::query_scalar(
            "INSERT INTO isahl_auth.auth_users (name, email, password_hash, status)
             VALUES ($1, $2, 'argon2-placeholder', 'active') RETURNING id",
        )
        .bind(format!("pc_test_{}", suffix))
        .bind(format!("pc_test_{}@example.com", suffix))
        .fetch_one(&pool)
        .await
        .expect("插入测试用户失败");

        let token = format!("sess_pc_test_{}", suffix);
        sqlx::query(
            "INSERT INTO isahl_auth.sso_sessions (user_id, session_token, status, expires_at)
             VALUES ($1, $2, 'active', NOW() + INTERVAL '1 hour')",
        )
        .bind(user_id)
        .bind(&token)
        .execute(&pool)
        .await
        .expect("插入测试 session 失败");

        // 前置断言：吊销前 token 仍活跃
        assert!(
            is_session_active(&pool, &token).await,
            "前置条件失败：session 应为 active"
        );

        // 模拟改密成功后新增的吊销逻辑
        let session_manager = SessionManager::new(pool.clone());
        let revoked = session_manager
            .revoke_all_user_sessions(user_id, None, Some(user_id), "password_change")
            .await
            .expect("吊销 session 失败");
        assert!(revoked >= 1, "应至少吊销 1 个 session");

        // 后置断言：旧 token 被 is_session_active 拒绝
        assert!(
            !is_session_active(&pool, &token).await,
            "改密后旧 session token 应被拒绝"
        );

        // 清理
        let _ = sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await;
    }
}
