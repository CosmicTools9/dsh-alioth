//! Admin 实名认证（identity verification）审核 handlers

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Identity verification response
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct IdentityVerificationResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub verification_type: Option<String>,
    pub verification_status: Option<String>,
    pub real_name: Option<String>,
    pub id_card_number: Option<String>,
    pub enterprise_name: Option<String>,
    pub business_license_number: Option<String>,
    pub rejected_reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Reject identity request
#[derive(Debug, Deserialize)]
pub struct RejectIdentityRequest {
    pub reason: String,
}

use super::require_admin;
use crate::auth::AuthState;

// ============================================================================
// Identity Verifications
// ============================================================================

/// GET /api/admin/identity-verifications?status=submitted
/// List identity verification records, optionally filtered by status.
pub async fn list_identity_verifications(
    req: HttpRequest,
    query: web::Query<StatusFilterParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let records = match query.status.as_deref() {
        Some(status) if !status.is_empty() => {
            sqlx::query_as::<_, IdentityVerificationResponse>(
                r#"
                SELECT id, user_id, verification_type, verification_status, real_name,
                       id_card_number, enterprise_name, business_license_number,
                       rejected_reason, created_at, updated_at
                FROM isahl_auth.identity_verifications
                WHERE verification_status = $1
                ORDER BY created_at DESC
                "#,
            )
            .bind(status)
            .fetch_all(pool.get_ref())
            .await
        }
        _ => {
            sqlx::query_as::<_, IdentityVerificationResponse>(
                r#"
                SELECT id, user_id, verification_type, verification_status, real_name,
                       id_card_number, enterprise_name, business_license_number,
                       rejected_reason, created_at, updated_at
                FROM isahl_auth.identity_verifications
                ORDER BY created_at DESC
                "#,
            )
            .fetch_all(pool.get_ref())
            .await
        }
    };

    match records {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({"identity_verifications": rows})),
        Err(e) => {
            log::error!("list_identity_verifications DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list identity verifications"}))
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct StatusFilterParams {
    pub status: Option<String>,
}

/// POST /api/admin/identity-verifications/{id}/approve
/// Approve an identity verification record.
pub async fn approve_identity_verification(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let verification_id = path.into_inner();

    let result = sqlx::query(
        r#"
        UPDATE isahl_auth.identity_verifications
        SET verification_status = 'approved',
            verified_at = NOW(),
            updated_at = NOW()
        WHERE id = $1 AND verification_status IN ('submitted', 'pending')
        "#,
    )
    .bind(verification_id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok()
            .json(serde_json::json!({"status": "approved", "id": verification_id})),
        Ok(_) => HttpResponse::NotFound()
            .json(serde_json::json!({"error": "Verification not found or already processed"})),
        Err(e) => {
            log::error!("approve_identity_verification DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to approve verification"}))
        }
    }
}

/// POST /api/admin/identity-verifications/{id}/reject
/// Reject an identity verification record.
pub async fn reject_identity_verification(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<RejectIdentityRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let verification_id = path.into_inner();

    let result = sqlx::query(
        r#"
        UPDATE isahl_auth.identity_verifications
        SET verification_status = 'rejected',
            rejected_reason = $1,
            updated_at = NOW()
        WHERE id = $2 AND verification_status IN ('submitted', 'pending')
        "#,
    )
    .bind(&body.reason)
    .bind(verification_id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok()
            .json(serde_json::json!({"status": "rejected", "id": verification_id})),
        Ok(_) => HttpResponse::NotFound()
            .json(serde_json::json!({"error": "Verification not found or already processed"})),
        Err(e) => {
            log::error!("reject_identity_verification DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to reject verification"}))
        }
    }
}
