//! 手机短信验证码认证 handlers

use actix_web::{web, HttpResponse};
use chrono::{Duration, Utc};
use common::{AliothError, ApiResponse, SmsService};
use rand::RngExt;
use serde::Deserialize;
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
pub struct SendCodeRequest {
    pub phone: String,
    #[serde(default = "default_purpose")]
    pub purpose: String,
}

fn default_purpose() -> String {
    "register".to_string()
}

#[derive(Debug, Deserialize)]
pub struct VerifyCodeRequest {
    pub phone: String,
    pub code: String,
    #[serde(default = "default_purpose")]
    pub purpose: String,
}

/// 生成 6 位数字验证码
fn generate_code() -> String {
    let mut rng = rand::rng();
    format!("{:06}", rng.random_range(0..1_000_000))
}

/// POST /auth/phone/send-code
pub async fn send_code(
    pool: web::Data<PgPool>,
    sms_service: web::Data<Box<dyn SmsService>>,
    body: web::Json<SendCodeRequest>,
) -> Result<HttpResponse, AliothError> {
    // 基础手机号格式校验（中国大陆 11 位）
    if body.phone.len() != 11 || !body.phone.chars().all(|c| c.is_ascii_digit()) {
        return Err(AliothError::BadRequest("Invalid phone format".to_string()));
    }

    let code = generate_code();
    let expires_at = Utc::now() + Duration::minutes(15);

    // 删除该手机号同一用途的未过期旧记录
    let delete_result = sqlx::query(
        "DELETE FROM isahl_auth.auth_phone_verifications WHERE phone = $1 AND purpose = $2 AND verified = FALSE"
    )
    .bind(&body.phone)
    .bind(&body.purpose)
    .execute(pool.get_ref())
    .await;

    if let Err(e) = delete_result {
        log::error!("Failed to clean old verification codes: {}", e);
    }

    // 写入新验证码
    let insert_result = sqlx::query(
        "INSERT INTO isahl_auth.auth_phone_verifications (phone, code, purpose, expires_at) VALUES ($1, $2, $3, $4)"
    )
    .bind(&body.phone)
    .bind(&code)
    .bind(&body.purpose)
    .bind(expires_at)
    .execute(pool.get_ref())
    .await;

    if let Err(e) = insert_result {
        log::error!("Failed to save verification code: {}", e);
        return Err(AliothError::Internal("Database error".to_string()));
    }

    // 发送短信
    let template_code = match body.purpose.as_str() {
        "register" => "SMS_12345678",
        "reset_password" => "SMS_87654321",
        _ => "SMS_12345678",
    };
    let params = serde_json::json!({"code": code}).to_string();

    match sms_service.send(&body.phone, template_code, &params).await {
        Ok(()) => {
            log::info!("SMS verification code sent to {}", body.phone);
            Ok(
                HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "sent": true,
                    "expires_in_minutes": 15
                }))),
            )
        }
        Err(e) => {
            log::error!("Failed to send SMS to {}: {}", body.phone, e);
            Err(AliothError::Internal("SMS service unavailable".to_string()))
        }
    }
}

/// POST /auth/phone/verify-code
pub async fn verify_code(
    pool: web::Data<PgPool>,
    body: web::Json<VerifyCodeRequest>,
) -> Result<HttpResponse, AliothError> {
    let row = sqlx::query_as::<_, (i64, String, bool)>(
        r#"
        SELECT id, code, verified
        FROM isahl_auth.auth_phone_verifications
        WHERE phone = $1
          AND purpose = $2
          AND expires_at > NOW()
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&body.phone)
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
                "UPDATE isahl_auth.auth_phone_verifications SET verified = TRUE, updated_at = NOW() WHERE id = $1"
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

/// Configure SMS auth routes（在 /auth scope 内注册）
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/phone/send-code", web::post().to(send_code))
        .route("/phone/verify-code", web::post().to(verify_code));
}
