//! 邮箱验证码认证 handlers

use actix_web::{web, HttpResponse};
use chrono::{Duration, Utc};
use common::{AliothError, ApiResponse, EmailService};
use rand::RngExt;
use serde::Deserialize;
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
pub struct SendCodeRequest {
    pub email: String,
    #[serde(default = "default_purpose")]
    pub purpose: String,
}

fn default_purpose() -> String {
    "register".to_string()
}

#[derive(Debug, Deserialize)]
pub struct VerifyCodeRequest {
    pub email: String,
    pub code: String,
    #[serde(default = "default_purpose")]
    pub purpose: String,
}

/// 生成 6 位数字验证码
fn generate_code() -> String {
    let mut rng = rand::rng();
    format!("{:06}", rng.random_range(0..1_000_000))
}

/// POST /auth/email/send-code
pub async fn send_code(
    pool: web::Data<PgPool>,
    email_service: web::Data<Box<dyn EmailService>>,
    body: web::Json<SendCodeRequest>,
) -> Result<HttpResponse, AliothError> {
    // 基础邮箱格式校验
    if !body.email.contains('@') || body.email.len() < 3 {
        return Err(AliothError::BadRequest("Invalid email format".to_string()));
    }

    let code = generate_code();
    let expires_at = Utc::now() + Duration::minutes(15);

    // 删除该邮箱同一用途的未过期旧记录
    let delete_result = sqlx::query(
        "DELETE FROM isahl_auth.auth_email_verifications WHERE email = $1 AND purpose = $2 AND verified = FALSE"
    )
    .bind(&body.email)
    .bind(&body.purpose)
    .execute(pool.get_ref())
    .await;

    if let Err(e) = delete_result {
        log::error!("Failed to clean old verification codes: {}", e);
    }

    // 写入新验证码
    let insert_result = sqlx::query(
        "INSERT INTO isahl_auth.auth_email_verifications (email, code, purpose, expires_at) VALUES ($1, $2, $3, $4)"
    )
    .bind(&body.email)
    .bind(&code)
    .bind(&body.purpose)
    .bind(expires_at)
    .execute(pool.get_ref())
    .await;

    if let Err(e) = insert_result {
        log::error!("Failed to save verification code: {}", e);
        return Err(AliothError::Internal("Database error".to_string()));
    }

    // 发送邮件
    let subject = match body.purpose.as_str() {
        "register" => "您的注册验证码",
        "reset_password" => "您的密码重置验证码",
        _ => "您的验证码",
    };
    let body_text = format!("您的验证码是：{}，15 分钟内有效。请勿泄露给他人。", code);

    match email_service.send(&body.email, subject, &body_text).await {
        Ok(()) => {
            log::info!("Verification code sent to {}", body.email);
            Ok(
                HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "sent": true,
                    "expires_in_minutes": 15
                }))),
            )
        }
        Err(e) => {
            log::error!("Failed to send email to {}: {}", body.email, e);
            // F-2 dev 降级：无 SMTP 环境验证码已落库，经 SSO_EMAIL_DEV_CODE=1 开关回传（仅 dev）
            if std::env::var("SSO_EMAIL_DEV_CODE").unwrap_or_default() == "1" {
                log::warn!("[DEV] verification code for {}: {}", body.email, code);
                return Ok(
                    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                        "sent": false,
                        "dev_code": code,
                        "expires_in_minutes": 15
                    }))),
                );
            }
            Err(AliothError::Internal(
                "Email service unavailable".to_string(),
            ))
        }
    }
}

/// POST /auth/email/verify-code
pub async fn verify_code(
    pool: web::Data<PgPool>,
    body: web::Json<VerifyCodeRequest>,
) -> Result<HttpResponse, AliothError> {
    let row = sqlx::query_as::<_, (i64, String, bool)>(
        r#"
        SELECT id, code, verified
        FROM isahl_auth.auth_email_verifications
        WHERE email = $1
          AND purpose = $2
          AND expires_at > NOW()
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&body.email)
    .bind(&body.purpose)
    .fetch_optional(pool.get_ref())
    .await;

    match row {
        Ok(Some((id, stored_code, verified))) => {
            if verified {
                return Err(AliothError::BadRequest("Code already used".to_string()));
            }
            if stored_code != body.code {
                return Err(AliothError::BadRequest("Invalid code".to_string()));
            }

            // 标记为已验证
            let update = sqlx::query(
                "UPDATE isahl_auth.auth_email_verifications SET verified = TRUE, updated_at = NOW() WHERE id = $1"
            )
            .bind(id)
            .execute(pool.get_ref())
            .await;

            if let Err(e) = update {
                log::error!("Failed to mark code as verified: {}", e);
                return Err(AliothError::Internal("Database error".to_string()));
            }

            Ok(
                HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "verified": true
                }))),
            )
        }
        Ok(None) => Err(AliothError::BadRequest(
            "Code expired or not found".to_string(),
        )),
        Err(e) => {
            log::error!("Failed to query verification code: {}", e);
            Err(AliothError::Internal("Database error".to_string()))
        }
    }
}

/// Configure email auth routes（在 /auth scope 内注册）
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/email/send-code", web::post().to(send_code))
        .route("/email/verify-code", web::post().to(verify_code));
}
