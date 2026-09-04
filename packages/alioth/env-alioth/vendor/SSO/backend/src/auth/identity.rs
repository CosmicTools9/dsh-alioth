//! Identity verification handler

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::jwt;
use super::AuthState;

#[derive(Debug, Deserialize)]
pub struct SubmitIdentityRequest {
    pub verification_type: String,
    pub real_name: Option<String>,
    pub id_card_number: Option<String>,
    pub id_card_front_url: Option<String>,
    pub id_card_back_url: Option<String>,
    pub enterprise_name: Option<String>,
    pub business_license_number: Option<String>,
    pub business_license_url: Option<String>,
    pub legal_person_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IdentityStatusResponse {
    pub verification_status: String,
    pub user_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "common::serde_zuid::opt", default)]
    pub approval_event_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthError {
    pub error: String,
}

pub async fn submit_identity(
    req: HttpRequest,
    body: web::Json<SubmitIdentityRequest>,
    pool: web::Data<PgPool>,
    auth_state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, auth_state.get_ref()).await {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or missing token".to_string(),
            });
        }
    };

    // Create entity instance based on verification type
    let instance_id = match body.verification_type.as_str() {
        "personal" => {
            sqlx::query_scalar::<_, i64>(
                r#"
                INSERT INTO isahl."zc_id_empl-natural" (
                    notice, code, fk_user, created_at, updated_at
                ) VALUES ($1, $2, $3, NOW(), NOW())
                RETURNING id
                "#,
            )
            .bind(&body.real_name)
            .bind(&body.id_card_number)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await
        }
        "enterprise" => {
            sqlx::query_scalar::<_, i64>(
                r#"
                INSERT INTO isahl."zc_id_orga-non-banking-legal" (
                    notice, created_at, updated_at
                ) VALUES ($1, NOW(), NOW())
                RETURNING id
                "#,
            )
            .bind(&body.enterprise_name)
            .fetch_one(pool.get_ref())
            .await
        }
        _ => {
            return HttpResponse::BadRequest().json(AuthError {
                error: "Invalid verification_type".to_string(),
            });
        }
    };

    let instance_id = match instance_id {
        Ok(id) => id,
        Err(e) => {
            log::error!("Failed to create entity instance: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to create entity instance".to_string(),
            });
        }
    };

    let result = sqlx::query(
        r#"
        INSERT INTO isahl_auth.identity_verifications (
            user_id, verification_type, real_name, id_card_number,
            id_card_front_url, id_card_back_url, enterprise_name,
            business_license_number, business_license_url, legal_person_name,
            entity_instance_id, entity_instance_table, verification_status,
            created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'submitted', NOW(), NOW())
        ON CONFLICT (user_id) DO UPDATE SET
            verification_type = EXCLUDED.verification_type,
            real_name = EXCLUDED.real_name,
            id_card_number = EXCLUDED.id_card_number,
            id_card_front_url = EXCLUDED.id_card_front_url,
            id_card_back_url = EXCLUDED.id_card_back_url,
            enterprise_name = EXCLUDED.enterprise_name,
            business_license_number = EXCLUDED.business_license_number,
            business_license_url = EXCLUDED.business_license_url,
            legal_person_name = EXCLUDED.legal_person_name,
            entity_instance_id = EXCLUDED.entity_instance_id,
            entity_instance_table = EXCLUDED.entity_instance_table,
            verification_status = 'submitted',
            updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(&body.verification_type)
    .bind(&body.real_name)
    .bind(&body.id_card_number)
    .bind(&body.id_card_front_url)
    .bind(&body.id_card_back_url)
    .bind(&body.enterprise_name)
    .bind(&body.business_license_number)
    .bind(&body.business_license_url)
    .bind(&body.legal_person_name)
    .bind(instance_id)
    .bind(match body.verification_type.as_str() {
        "personal" => "zc_id_empl-natural",
        "enterprise" => "zc_id_orga-non-banking-legal",
        _ => "",
    })
    .fetch_one(pool.get_ref())
    .await;

    match result {
        Ok(_) => {
            let _ = sqlx::query(
                "UPDATE isahl_auth.auth_users SET status = 'identity_submitted', updated_at = NOW() WHERE id = $1"
            )
            .bind(user_id)
            .execute(pool.get_ref())
            .await;

            HttpResponse::Ok().json(serde_json::json!({
                "status": "submitted",
                "instance_id": instance_id,
            }))
        }
        Err(e) => {
            log::error!("Failed to submit identity: {}", e);
            HttpResponse::InternalServerError().json(AuthError {
                error: "Failed to submit identity".to_string(),
            })
        }
    }
}

pub async fn get_identity_status(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth_state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, auth_state.get_ref()).await {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or missing token".to_string(),
            });
        }
    };

    let row = sqlx::query_as::<_, (Option<String>, Option<String>, Option<i64>, Option<String>)>(
        r#"
        SELECT iv.verification_status, u.status, iv.approval_event_id, ast.code AS approval_status
        FROM isahl_auth.auth_users u
        LEFT JOIN isahl_auth.identity_verifications iv ON iv.user_id = u.id
        LEFT JOIN isahl."zc_id_lifecycle_r_primary-status" ps ON ps.ref_left = iv.approval_event_id AND ps.deleted_at IS NULL
        LEFT JOIN isahl."zc_id_stus-approve" ast ON ast.id = ps.ref_right AND ast.deleted_at IS NULL
        WHERE u.id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await;

    match row {
        Ok(Some((ver_status, user_status, approval_event_id, approval_status))) => {
            HttpResponse::Ok().json(IdentityStatusResponse {
                verification_status: ver_status.unwrap_or_else(|| "not_submitted".to_string()),
                user_status: user_status.unwrap_or_else(|| "unknown".to_string()),
                approval_event_id,
                approval_status,
            })
        }
        Ok(None) => HttpResponse::NotFound().json(AuthError {
            error: "User not found".to_string(),
        }),
        Err(e) => {
            log::error!("Database error: {}", e);
            HttpResponse::InternalServerError().json(AuthError {
                error: "Database error".to_string(),
            })
        }
    }
}

pub async fn verify_identity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth_state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, auth_state.get_ref()).await {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or missing token".to_string(),
            });
        }
    };

    let identity = match sqlx::query_as::<_, (i64, i64, String)>(
        r#"
        SELECT id, entity_instance_id, entity_instance_table
        FROM isahl_auth.identity_verifications
        WHERE user_id = $1 AND verification_status = 'submitted'
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return HttpResponse::BadRequest().json(AuthError {
                error: "No pending identity verification found".to_string(),
            });
        }
        Err(e) => {
            log::error!("Database error: {}", e);
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Database error".to_string(),
            });
        }
    };

    // 身份证实：依据配置的模式实际核验，而非盲目通过。
    // - local（默认）：校验提交数据的完整性（必填字段齐全）；
    // - external：调用第三方 API（IDENTITY_EXTERNAL_VERIFY_URL），
    //   未配置 URL 时失败封闭，避免静默放行。
    let verified = match auth_state.identity_verify_mode.as_str() {
        "external" => match &auth_state.identity_external_verify_url {
            Some(url) => verify_identity_external(url, pool.get_ref(), identity.0).await,
            None => {
                log::error!(
                    "identity_verify_mode=external 但 IDENTITY_EXTERNAL_VERIFY_URL 未配置，拒绝核验"
                );
                false
            }
        },
        _ => verify_identity_locally(pool.get_ref(), identity.0).await,
    };

    if verified {
        let _ = sqlx::query(
            "UPDATE isahl_auth.identity_verifications SET verification_status = 'verified', verified_at = NOW(), updated_at = NOW() WHERE id = $1"
        )
        .bind(identity.0)
        .execute(pool.get_ref())
        .await;

        // 审批事件创建（add-register-auto-approval 修复）：
        // - 注册路径已自动触发审批（auth_users.status='pending_approval'）→ 幂等跳过，
        //   避免双重审批事件（注册审批经 oper-approve 实例 + comments 关联，不依赖本列）；
        // - 旧路径（未注册直接建用户）→ 用存在列创建（原 ck_category/fk_object 列不存在，
        //   必失败并静默降级 approval_event_id=0——断链根源）。
        let user_status: Option<String> =
            sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(pool.get_ref())
                .await
                .ok()
                .flatten();

        let approval_event_id: i64 = if user_status.as_deref() == Some("pending_approval") {
            sqlx::query_scalar(
                "SELECT COALESCE(MAX(approval_event_id), 0) FROM isahl_auth.identity_verifications WHERE user_id = $1",
            )
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await
            .unwrap_or(0)
        } else {
            // add-approval-leaf-template-seeds：实名审核事件落叶表并绑 FLOW-USER-VERIFY 模板
            // （fk_process/tpl_id/_f_/_t_/qk_sla；模板缺失降级 NULL，seed 组件一致性回填）
            let verify_binding: Option<(i64, Option<i64>)> = sqlx::query_as(
                r#"
                SELECT p.id,
                       (SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
                        WHERE rro.ref_left = p.id AND rro.code = 'approve'
                          AND rro.deleted_at IS NULL LIMIT 1)
                FROM isahl.zc_id_process p
                WHERE p.code = 'FLOW-USER-VERIFY' AND p.deleted_at IS NULL
                LIMIT 1
                "#,
            )
            .fetch_optional(pool.get_ref())
            .await
            .ok()
            .flatten();
            let sla_duration_id: Option<i64> = sqlx::query_scalar(
                r#"SELECT id FROM isahl."zc_id_scal-duration"
                   WHERE o_number = '72h' AND deleted_at IS NULL LIMIT 1"#,
            )
            .fetch_optional(pool.get_ref())
            .await
            .ok()
            .flatten();
            sqlx::query_scalar(
                r#"
                INSERT INTO isahl."zc_id_appr-user_verify" (
                    created_by_id, updated_by_id, notice, code, comments,
                    fk_process, tpl_id, qk_sla, _f_, _t_, created_at, updated_at
                ) VALUES ($1, $1, $2, 'user-verify', $3, $4, $5, $6, '实现', '实例', NOW(), NOW())
                RETURNING id
                "#,
            )
            .bind(user_id)
            .bind(format!("用户 {} 实名审核", user_id))
            .bind(
                serde_json::json!({
                    "entity_instance_id": identity.1,
                    "entity_instance_table": identity.2,
                })
                .to_string(),
            )
            .bind(verify_binding.as_ref().map(|(flow_id, _)| *flow_id))
            .bind(verify_binding.as_ref().and_then(|(_, tpl)| *tpl))
            .bind(sla_duration_id)
            .fetch_one(pool.get_ref())
            .await
            .unwrap_or(0)
        };

        let _ = sqlx::query(
            "UPDATE isahl_auth.identity_verifications SET approval_event_id = $1, updated_at = NOW() WHERE id = $2"
        )
        .bind(approval_event_id)
        .bind(identity.0)
        .execute(pool.get_ref())
        .await;

        let _ = sqlx::query(
            "UPDATE isahl_auth.auth_users SET status = 'pending_approval', updated_at = NOW() WHERE id = $1"
        )
        .bind(user_id)
        .execute(pool.get_ref())
        .await;

        HttpResponse::Ok().json(serde_json::json!({
            "status": "verified",
            "approval_event_id": approval_event_id,
        }))
    } else {
        // 身份验证失败：回退到 pending 状态，允许用户重试
        let _ = sqlx::query(
            "UPDATE isahl_auth.identity_verifications SET verification_status = 'rejected', rejected_reason = 'Third-party verification failed', updated_at = NOW() WHERE id = $1"
        )
        .bind(identity.0)
        .execute(pool.get_ref())
        .await;

        let _ = sqlx::query(
            "UPDATE isahl_auth.auth_users SET status = 'pending', updated_at = NOW() WHERE id = $1",
        )
        .bind(user_id)
        .execute(pool.get_ref())
        .await;

        HttpResponse::Ok().json(serde_json::json!({
            "status": "rejected",
        }))
    }
}

/// 本地核验：检查该用户的身份提交记录关键字段是否齐全。
/// 不完整 → 返回 false（拒绝核验），避免空数据被自动通过。
async fn verify_identity_locally(pool: &PgPool, verification_id: i64) -> bool {
    let row = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT verification_type, real_name, id_card_number,
               id_card_front_url, id_card_back_url, enterprise_name,
               business_license_number, business_license_url, legal_person_name
        FROM isahl_auth.identity_verifications
        WHERE id = $1
        "#,
    )
    .bind(verification_id)
    .fetch_optional(pool)
    .await;

    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            log::error!(
                "verify_identity_locally: 未找到核验记录 id={}",
                verification_id
            );
            return false;
        }
        Err(e) => {
            log::error!("verify_identity_locally: 数据库错误: {}", e);
            return false;
        }
    };

    let (vtype, real_name, id_card, front, back, ent_name, lic_no, lic_url, legal) = row;

    match vtype.as_deref() {
        Some("personal") => {
            real_name.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                && id_card.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                && front.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                && back.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
        }
        Some("enterprise") => {
            ent_name.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                && lic_no.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                && lic_url.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                && legal.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
        }
        _ => false,
    }
}

/// 外部核验：将身份提交数据 POST 至第三方 API，解析其返回的 `verified` 布尔值。
/// 任何网络/解析失败 → 返回 false（失败封闭）。
async fn verify_identity_external(url: &str, pool: &PgPool, verification_id: i64) -> bool {
    let row = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT verification_type, real_name, id_card_number, enterprise_name
        FROM isahl_auth.identity_verifications
        WHERE id = $1
        "#,
    )
    .bind(verification_id)
    .fetch_optional(pool)
    .await;

    let payload = match row {
        Ok(Some(r)) => serde_json::json!({
            "verification_id": verification_id,
            "verification_type": r.0,
            "real_name": r.1,
            "id_card_number": r.2,
            "enterprise_name": r.3,
        }),
        Ok(None) => {
            log::error!(
                "verify_identity_external: 未找到核验记录 id={}",
                verification_id
            );
            return false;
        }
        Err(e) => {
            log::error!("verify_identity_external: 数据库错误: {}", e);
            return false;
        }
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            log::error!("verify_identity_external: 请求失败: {}", e);
            return false;
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("verify_identity_external: 响应解析失败: {}", e);
            return false;
        }
    };

    body.get("verified")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

async fn extract_user_id(req: &HttpRequest, auth_state: &AuthState) -> Result<i64, &'static str> {
    let token = req
        .cookie("access_token")
        .map(|c| c.value().to_string())
        .or_else(|| {
            req.headers()
                .get(actix_web::http::header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|auth| auth.strip_prefix("Bearer "))
                .map(|s| s.to_string())
        });

    let token = token.ok_or("missing token")?;

    // 验签解码（ES256 + exp/iss/aud 校验）后从 sub 提取用户 id（add-register-auto-approval）：
    // 历史实现把原始 JWT 字符串绑定到 `u.id::text = $1` 恒不匹配，导致 /auth/identity/* 全部 401。
    let claims = jwt::decode_token_any(&token, &auth_state.verification_keys())
        .map_err(|_| "invalid token")?;
    claims.sub.parse::<i64>().map_err(|_| "invalid sub")
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth/identity")
            .route("/submit", web::post().to(submit_identity))
            .route("/status", web::get().to(get_identity_status))
            .route("/verify", web::post().to(verify_identity)),
    );
}
