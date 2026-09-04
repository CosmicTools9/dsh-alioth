//! 权限申请闭环（add-ngac-access-request）
//!
//! 语义：
//! - 本人端点（`/api/ngac/access-request*`，JWT 强制本人）：发起申请 / 我的申请列表。
//! - 管理端点（`/api/admin/ngac/access-requests*`，require_admin）：列表 / 审批通过
//!   （复用 `assign_ua_with_audit_tx`，同一事务内指派 + 审计 + 状态流转）/ 拒绝。
//! - 状态机：pending → approved | rejected；已审结幂等 409。
//! - 表 `isahl_auth.ngac_access_request`（运行时幂等 ensure：`ngac/ensure.rs`）。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::ngac::pip::assign_ua_with_audit_tx;

/// 本人发起申请请求体。
#[derive(Debug, Deserialize)]
pub struct CreateAccessRequest {
    pub resource_type: String,
    pub action: String,
    pub reason: Option<String>,
}

/// 审批通过请求体。
#[derive(Debug, Deserialize)]
pub struct ApproveAccessRequest {
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    pub expires_at: Option<DateTime<Utc>>,
}

/// 拒绝请求体。
#[derive(Debug, Deserialize)]
pub struct RejectAccessRequest {
    pub reason: Option<String>,
}

/// 列表查询参数。
#[derive(Debug, Deserialize)]
pub struct AccessRequestQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AccessRequestRow {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_user: i64,
    pub resource_type: String,
    pub action: String,
    pub reason: Option<String>,
    pub status: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_assignee_ua: Option<i64>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub reviewed_by: Option<i64>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

const ALLOWED_STATUS: [&str; 3] = ["pending", "approved", "rejected"];

fn status_clause(status: &str) -> &'static str {
    match status {
        "approved" => " AND status = 'approved'",
        "rejected" => " AND status = 'rejected'",
        _ => " AND status = 'pending'",
    }
}

/// 从 JWT 取本人 user_id（与 review/me 同模式）。
async fn jwt_user_id(
    req: &HttpRequest,
    state: &crate::auth::AuthState,
) -> Result<i64, HttpResponse> {
    let claims =
        match crate::auth::jwt::validate_access_token(req, &state.verification_keys()).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("access_request: token validation failed: {}", e);
                return Err(HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": "Invalid or missing authentication token"
                })));
            }
        };
    claims.sub.parse().map_err(|_| {
        HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Invalid token subject"
        }))
    })
}

/// POST /api/ngac/access-request — 本人发起申请（pending 幂等 409）
pub async fn create_access_request(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    body: web::Json<CreateAccessRequest>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let user_id = match jwt_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let body = body.into_inner();
    if body.resource_type.trim().is_empty() || body.action.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "resource_type and action are required"
        }));
    }

    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("create_access_request: tx begin failed: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create access request"
            }));
        }
    };
    // 幂等防刷：同用户同 (resource_type, action) 未决 pending → 409
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_request \
         WHERE fk_user = $1 AND resource_type = $2 AND action = $3 AND status = 'pending' \
           AND deleted_at IS NULL LIMIT 1",
    )
    .bind(user_id)
    .bind(&body.resource_type)
    .bind(&body.action)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);
    if existing.is_some() {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": "Pending access request already exists for this resource and action"
        }));
    }
    let result: sqlx::Result<i64> = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_access_request (fk_user, resource_type, action, reason) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(user_id)
    .bind(&body.resource_type)
    .bind(&body.action)
    .bind(&body.reason)
    .fetch_one(&mut *tx)
    .await;
    match result {
        Ok(id) => {
            if let Err(e) = tx.commit().await {
                log::error!("create_access_request: commit failed: {}", e);
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "error": "Failed to create access request"
                }));
            }
            HttpResponse::Created().json(serde_json::json!({
                "id": id.to_string(),
                "status": "pending",
            }))
        }
        Err(e) => {
            log::error!("create_access_request: insert failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create access request"
            }))
        }
    }
}

/// GET /api/ngac/access-request/me — 我的申请列表（分页）
pub async fn list_my_access_requests(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    query: web::Query<AccessRequestQuery>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let user_id = match jwt_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let status_filter = query
        .status
        .as_deref()
        .filter(|s| ALLOWED_STATUS.contains(s));

    let rows = if let Some(s) = status_filter {
        sqlx::query_as::<_, AccessRequestRow>(
            "SELECT id, fk_user, resource_type, action, reason, status, fk_assignee_ua, \
                    expires_at, reviewed_by, reviewed_at, created_at \
             FROM isahl_auth.ngac_access_request \
             WHERE fk_user = $1 AND status = $2 AND deleted_at IS NULL \
             ORDER BY id DESC LIMIT $3 OFFSET $4",
        )
        .bind(user_id)
        .bind(s)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await
    } else {
        sqlx::query_as::<_, AccessRequestRow>(
            "SELECT id, fk_user, resource_type, action, reason, status, fk_assignee_ua, \
                    expires_at, reviewed_by, reviewed_at, created_at \
             FROM isahl_auth.ngac_access_request \
             WHERE fk_user = $1 AND deleted_at IS NULL \
             ORDER BY id DESC LIMIT $2 OFFSET $3",
        )
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await
    };
    match rows {
        Ok(rows) => HttpResponse::Ok().json(rows),
        Err(e) => {
            log::error!("list_my_access_requests: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to list access requests"
            }))
        }
    }
}

/// GET /api/admin/ngac/access-requests — 管理列表（require_admin，缺省 pending）
pub async fn list_access_requests(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    query: web::Query<AccessRequestQuery>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    if let Err(resp) =
        crate::admin::handlers::require_admin(&req, pool.get_ref(), state.get_ref()).await
    {
        return resp;
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let clause = status_clause(query.status.as_deref().unwrap_or("pending"));

    let sql = format!(
        "SELECT id, fk_user, resource_type, action, reason, status, fk_assignee_ua, \
                expires_at, reviewed_by, reviewed_at, created_at \
         FROM isahl_auth.ngac_access_request \
         WHERE deleted_at IS NULL{clause} \
         ORDER BY id DESC LIMIT $1 OFFSET $2"
    );
    let rows = sqlx::query_as::<_, AccessRequestRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await;
    match rows {
        Ok(rows) => HttpResponse::Ok().json(rows),
        Err(e) => {
            log::error!("list_access_requests: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to list access requests"
            }))
        }
    }
}

/// POST /api/admin/ngac/access-requests/{id}/approve — 审批通过（指派 + 审计 + 状态）
pub async fn approve_access_request(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
    body: web::Json<ApproveAccessRequest>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let admin_id =
        match crate::admin::handlers::require_admin(&req, pool.get_ref(), state.get_ref()).await {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let request_id = path.into_inner();
    let body = body.into_inner();

    // UA 存在性校验
    let ua_exists: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(body.fk_user_attribute)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);
    if ua_exists.is_none() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Target user attribute not found"
        }));
    }

    let session_id = crate::auth::jwt::validate_access_token(&req, &state.verification_keys())
        .await
        .ok()
        .and_then(|c| if c.sid.is_empty() { None } else { Some(c.sid) });
    let ip = crate::ngac::audit_writer::client_ip(&req);

    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("approve_access_request: tx begin failed: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to approve access request"
            }));
        }
    };
    // 已审结 → 409（在事务内锁定行）
    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, status FROM isahl_auth.ngac_access_request \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);
    let Some((_, status)) = row else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "Access request not found"
        }));
    };
    if status != "pending" {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": format!("Access request already {}", status)
        }));
    }
    let applicant: i64 =
        sqlx::query_scalar("SELECT fk_user FROM isahl_auth.ngac_access_request WHERE id = $1")
            .bind(request_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap_or(0);

    let result: sqlx::Result<()> = async {
        assign_ua_with_audit_tx(
            &mut tx,
            applicant,
            body.fk_user_attribute,
            None,
            body.expires_at,
            &crate::ngac::pip::AuditContext {
                actor: admin_id,
                session_id,
                ip_address: ip,
            },
        )
        .await?;
        sqlx::query(
            "UPDATE isahl_auth.ngac_access_request \
             SET status = 'approved', fk_assignee_ua = $2, expires_at = $3, \
                 reviewed_by = $4, reviewed_at = NOW(), updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(request_id)
        .bind(body.fk_user_attribute)
        .bind(body.expires_at)
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "id": request_id.to_string(),
            "status": "approved",
        })),
        Err(e) => {
            log::error!("approve_access_request: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to approve access request"
            }))
        }
    }
}

/// POST /api/admin/ngac/access-requests/{id}/reject — 拒绝（不创建任何指派）
pub async fn reject_access_request(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
    body: web::Json<RejectAccessRequest>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let admin_id =
        match crate::admin::handlers::require_admin(&req, pool.get_ref(), state.get_ref()).await {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let request_id = path.into_inner();
    let body = body.into_inner();

    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("reject_access_request: tx begin failed: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to reject access request"
            }));
        }
    };
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM isahl_auth.ngac_access_request \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);
    let Some(status) = status else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "Access request not found"
        }));
    };
    if status != "pending" {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": format!("Access request already {}", status)
        }));
    }
    let result: sqlx::Result<()> = async {
        sqlx::query(
            "UPDATE isahl_auth.ngac_access_request \
             SET status = 'rejected', reason = COALESCE($2, reason), \
                 reviewed_by = $3, reviewed_at = NOW(), updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(request_id)
        .bind(&body.reason)
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "id": request_id.to_string(),
            "status": "rejected",
        })),
        Err(e) => {
            log::error!("reject_access_request: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to reject access request"
            }))
        }
    }
}

/// 管理面路由（挂载于 admin configure，RequireAuth + NgacPep 双重保护）
pub fn configure_admin_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/ngac/access-requests", web::get().to(list_access_requests))
        .route(
            "/ngac/access-requests/{id}/approve",
            web::post().to(approve_access_request),
        )
        .route(
            "/ngac/access-requests/{id}/reject",
            web::post().to(reject_access_request),
        );
}

/// 本人路由（挂载于 /api/ngac，PEP 层豁免 + handler 内 JWT 强制）
pub fn configure_self_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/access-request", web::post().to(create_access_request))
        .route("/access-request/me", web::get().to(list_my_access_requests));
}
