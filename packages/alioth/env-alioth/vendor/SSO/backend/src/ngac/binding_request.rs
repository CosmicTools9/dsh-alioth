//! 主体/岗位绑定自助申请（add-ngac-binding-request）
//!
//! 语义（与 access_request 同构，目标表不同）：
//! - 本人端点（`/api/ngac/binding-request*`，JWT 强制本人）：发起 / 我的申请。
//! - 管理端点（`/api/admin/ngac/binding-requests*`，require_admin）：列表 / 审批：
//!   - kind=entity：目标实体存在且属组织类白名单 → UPDATE auth_users
//!     entity_table/entity_id（m2o 语义，与 Gateway entity-binding 一致）；
//!   - kind=position：目标岗位存在 → 确保用户 `zc_id_empl-natural` 雇员行 →
//!     幂等创建 `zc_id_subj-post_rr_employee` 任职行（active 存在则跳过）。
//! - 表 `isahl_auth.ngac_binding_request`（运行时幂等 ensure：`ngac/ensure.rs`）。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// 组织类实体白名单（add-ngac-binding-request D3）——与 Gateway
/// `api/entity_binding.rs::is_org_table` 同源四值，跨 crate 复制 MUST 注释同步。
const ORG_TABLE_WHITELIST: [&str; 4] = [
    "zc_id_orga-non-banking-legal",
    "zc_id_subj-org",
    "zc_id_subjects",
    "zc_id_entity",
];

/// 发起绑定申请请求体。
#[derive(Debug, Deserialize)]
pub struct CreateBindingRequest {
    pub kind: String,
    #[serde(with = "common::serde_zuid")]
    pub target_id: i64,
    pub reason: Option<String>,
}

/// 列表查询参数。
#[derive(Debug, Deserialize)]
pub struct BindingQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct BindingRequestRow {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_user: i64,
    pub kind: String,
    #[serde(with = "common::serde_zuid")]
    pub target_id: i64,
    pub reason: Option<String>,
    pub status: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub reviewed_by: Option<i64>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

async fn jwt_user_id(
    req: &HttpRequest,
    state: &crate::auth::AuthState,
) -> Result<i64, HttpResponse> {
    let claims =
        match crate::auth::jwt::validate_access_token(req, &state.verification_keys()).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("binding_request: token validation failed: {}", e);
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

/// POST /api/ngac/binding-request — 本人发起绑定申请
pub async fn create_binding_request(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    body: web::Json<CreateBindingRequest>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let user_id = match jwt_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let body = body.into_inner();
    if !matches!(body.kind.as_str(), "entity" | "position") {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "kind must be 'entity' or 'position'"
        }));
    }
    // 幂等防刷：同用户同 (kind, target_id) 未决 pending → 409
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_binding_request \
         WHERE fk_user = $1 AND kind = $2 AND target_id = $3 AND status = 'pending' \
           AND deleted_at IS NULL LIMIT 1",
    )
    .bind(user_id)
    .bind(&body.kind)
    .bind(body.target_id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);
    if existing.is_some() {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": "Pending binding request already exists"
        }));
    }
    let result = sqlx::query_scalar::<_, i64>(
        "INSERT INTO isahl_auth.ngac_binding_request (fk_user, kind, target_id, reason) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(user_id)
    .bind(&body.kind)
    .bind(body.target_id)
    .bind(&body.reason)
    .fetch_one(pool.get_ref())
    .await;
    match result {
        Ok(id) => HttpResponse::Created().json(serde_json::json!({
            "id": id.to_string(),
            "status": "pending",
        })),
        Err(e) => {
            log::error!("create_binding_request: insert failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create binding request"
            }))
        }
    }
}

/// GET /api/ngac/binding-request/me — 我的申请列表
pub async fn list_my_binding_requests(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    query: web::Query<BindingQuery>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let user_id = match jwt_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let rows = sqlx::query_as::<_, BindingRequestRow>(
        "SELECT id, fk_user, kind, target_id, reason, status, reviewed_by, reviewed_at, created_at \
         FROM isahl_auth.ngac_binding_request \
         WHERE fk_user = $1 AND deleted_at IS NULL \
         ORDER BY id DESC LIMIT $2 OFFSET $3",
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await;
    match rows {
        Ok(rows) => HttpResponse::Ok().json(rows),
        Err(e) => {
            log::error!("list_my_binding_requests: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to list binding requests"
            }))
        }
    }
}

/// GET /api/admin/ngac/binding-requests — 管理列表（require_admin，缺省 pending）
pub async fn list_binding_requests(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    query: web::Query<BindingQuery>,
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
    let clause = match query.status.as_deref() {
        Some("approved") => "AND status = 'approved'",
        Some("rejected") => "AND status = 'rejected'",
        _ => "AND status = 'pending'",
    };
    let sql = format!(
        "SELECT id, fk_user, kind, target_id, reason, status, reviewed_by, reviewed_at, created_at \
         FROM isahl_auth.ngac_binding_request \
         WHERE deleted_at IS NULL {clause} \
         ORDER BY id DESC LIMIT $1 OFFSET $2"
    );
    let rows = sqlx::query_as::<_, BindingRequestRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await;
    match rows {
        Ok(rows) => HttpResponse::Ok().json(rows),
        Err(e) => {
            log::error!("list_binding_requests: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to list binding requests"
            }))
        }
    }
}

/// POST /api/admin/ngac/binding-requests/{id}/approve — 审批通过（同事务）
pub async fn approve_binding_request(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let admin_id =
        match crate::admin::handlers::require_admin(&req, pool.get_ref(), state.get_ref()).await {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let request_id = path.into_inner();

    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("approve_binding_request: tx begin failed: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to approve binding request"
            }));
        }
    };
    let row: Option<(i64, String, i64, String)> = sqlx::query_as(
        "SELECT fk_user, kind, target_id, status FROM isahl_auth.ngac_binding_request \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);
    let Some((applicant, kind, target_id, status)) = row else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "Binding request not found"
        }));
    };
    if status != "pending" {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": format!("Binding request already {}", status)
        }));
    }

    let result: sqlx::Result<()> = async {
        match kind.as_str() {
            "entity" => {
                // 目标实体存在 + 组织类白名单
                let entity: Option<(String,)> = sqlx::query_as(
                    "SELECT tableoid::regclass::text FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(target_id)
                .fetch_optional(&mut *tx)
                .await?;
                let Some((table,)) = entity else {
                    return Err(sqlx::Error::RowNotFound);
                };
                let table = table.trim_matches('"').replace("\"", "");
                let org_table = ORG_TABLE_WHITELIST
                    .iter()
                    .any(|t| table.ends_with(t) || table == *t);
                if !org_table {
                    return Err(sqlx::Error::RowNotFound);
                }
                sqlx::query(
                    "UPDATE isahl_auth.auth_users SET entity_id = $1, entity_table = $2, updated_at = NOW() WHERE id = $3",
                )
                .bind(target_id)
                .bind(&table)
                .bind(applicant)
                .execute(&mut *tx)
                .await?;
            }
            "position" => {
                // 岗位存在
                let pos: Option<(i64,)> = sqlx::query_as(
                    "SELECT id FROM isahl.\"zc_id_subj-position\" WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(target_id)
                .fetch_optional(&mut *tx)
                .await?;
                let Some(_) = pos else {
                    return Err(sqlx::Error::RowNotFound);
                };
                // 确保个人雇员行（empl-natural）
                let employee: Option<(i64,)> = sqlx::query_as(
                    "SELECT id FROM isahl.\"zc_id_empl-natural\" WHERE fk_user = $1 AND deleted_at IS NULL LIMIT 1",
                )
                .bind(applicant)
                .fetch_optional(&mut *tx)
                .await?;
                let employee_id: i64 = match employee {
                    Some((eid,)) => eid,
                    None => {
                        sqlx::query_scalar(
                            "INSERT INTO isahl.\"zc_id_empl-natural\" (id, notice, code, fk_user, created_by_id) \
                             VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1) RETURNING id",
                        )
                        .bind(format!("自动雇员-{}", applicant))
                        .bind(format!("AUTO-EMP-{}", applicant))
                        .bind(applicant)
                        .fetch_one(&mut *tx)
                        .await?
                    }
                };
                // 幂等任职：active 存在则跳过
                let existing: Option<(i64,)> = sqlx::query_as(
                    "SELECT id FROM isahl.\"zc_id_subj-post_rr_employee\" \
                     WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL LIMIT 1",
                )
                .bind(target_id)
                .bind(employee_id)
                .fetch_optional(&mut *tx)
                .await?;
                if existing.is_none() {
                    sqlx::query(
                        "INSERT INTO isahl.\"zc_id_subj-post_rr_employee\" (id, notice, ref_left, ref_right, created_by_id) \
                         VALUES (isahl.gen_next_uid(312), '绑定申请任职', $1, $2, 1)",
                    )
                    .bind(target_id)
                    .bind(employee_id)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            _ => {
                return Err(sqlx::Error::RowNotFound);
            }
        }
        sqlx::query(
            "UPDATE isahl_auth.ngac_binding_request \
             SET status = 'approved', reviewed_by = $2, reviewed_at = NOW(), updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(request_id)
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
        Err(sqlx::Error::RowNotFound) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Target entity/position not found or not an organization entity"
        })),
        Err(e) => {
            log::error!("approve_binding_request: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to approve binding request"
            }))
        }
    }
}

/// POST /api/admin/ngac/binding-requests/{id}/reject — 拒绝
pub async fn reject_binding_request(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let admin_id =
        match crate::admin::handlers::require_admin(&req, pool.get_ref(), state.get_ref()).await {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let request_id = path.into_inner();
    let reason = body
        .get("reason")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string());

    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("reject_binding_request: tx begin failed: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to reject binding request"
            }));
        }
    };
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM isahl_auth.ngac_binding_request \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);
    let Some(status) = status else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "Binding request not found"
        }));
    };
    if status != "pending" {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": format!("Binding request already {}", status)
        }));
    }
    let result: sqlx::Result<()> = async {
        sqlx::query(
            "UPDATE isahl_auth.ngac_binding_request \
             SET status = 'rejected', reason = COALESCE($2, reason), \
                 reviewed_by = $3, reviewed_at = NOW(), updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(request_id)
        .bind(&reason)
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
            log::error!("reject_binding_request: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to reject binding request"
            }))
        }
    }
}

/// 管理面路由
pub fn configure_admin_routes(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/ngac/binding-requests",
        web::get().to(list_binding_requests),
    )
    .route(
        "/ngac/binding-requests/{id}/approve",
        web::post().to(approve_binding_request),
    )
    .route(
        "/ngac/binding-requests/{id}/reject",
        web::post().to(reject_binding_request),
    );
}

/// 本人路由
pub fn configure_self_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/binding-request", web::post().to(create_binding_request))
        .route(
            "/binding-request/me",
            web::get().to(list_my_binding_requests),
        );
}
