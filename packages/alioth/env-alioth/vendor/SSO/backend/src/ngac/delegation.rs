//! 通用 NGAC 委托（add-ngac-delegation）
//!
//! 语义：
//! - 委托方（delegator）把本人**有效持有**的 UA（直接指派 ∪ 认知派生，排除委托派生
//!   防链式扩散）限时委托给被委托方（delegatee）。
//! - 生效/撤销均为**读侧派生**（PIP 基集 UNION `DELEGATED_CTE`），零物化指派行；
//!   撤销 = status→revoked，即时失效。
//! - 端点挂 `/api/ngac`（PEP 层豁免 + handler 内 JWT 强制本人，与 review/me 同模式）。
//! - 表 `isahl_auth.ngac_delegation`（运行时幂等 ensure：`ngac/ensure.rs`）。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::ngac::pip::{Pip, PostgresPip};

/// 发起委托请求体。
#[derive(Debug, Deserialize)]
pub struct CreateDelegationRequest {
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_delegatee: i64,
    pub date_st: DateTime<Utc>,
    pub date_ed: DateTime<Utc>,
}

/// 列表查询参数。
#[derive(Debug, Deserialize)]
pub struct DelegationQuery {
    pub direction: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DelegationRow {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_delegator: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_delegatee: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    pub date_st: DateTime<Utc>,
    pub date_ed: DateTime<Utc>,
    pub status: String,
    pub created_at: DateTime<Utc>,
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
                log::warn!("delegation: token validation failed: {}", e);
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

/// 委托方可直接委托的来源：直接指派 ∪ 认知派生（排除委托派生——防链式扩散）。
async fn delegatable_ua_ids(pool: &PgPool, user_id: i64) -> Vec<i64> {
    // 认知派生名 → id
    let pip = PostgresPip::new(pool.clone());
    let attrs = match pip.get_all_user_attributes_with_inheritance(user_id).await {
        Ok(a) => a,
        Err(e) => {
            log::warn!("delegation: effective attrs failed for {}: {}", user_id, e);
            Vec::new()
        }
    };
    let effective_ids: std::collections::HashSet<i64> = attrs.iter().map(|a| a.id).collect();
    // 委托派生 id（需排除）
    let delegated_ids: std::collections::HashSet<i64> = sqlx::query_scalar(
        "SELECT fk_user_attribute FROM isahl_auth.ngac_delegation \
         WHERE fk_delegatee = $1 AND status = 'active' AND deleted_at IS NULL \
           AND date_st <= NOW() AND date_ed > NOW()",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .collect();
    effective_ids.difference(&delegated_ids).copied().collect()
}

/// POST /api/ngac/delegations — 本人发起委托
pub async fn create_delegation(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    body: web::Json<CreateDelegationRequest>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let delegator = match jwt_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let body = body.into_inner();
    if body.date_ed <= body.date_st {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "date_ed must be after date_st"
        }));
    }
    if body.fk_delegatee == delegator {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Cannot delegate to self"
        }));
    }
    // 被委托方存在
    let delegatee_exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE id = $1")
            .bind(body.fk_delegatee)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None);
    if delegatee_exists.is_none() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Delegatee not found"
        }));
    }
    // 委托目标 ∈ 可直接委托来源（有效持有 - 委托派生）
    let delegatable = delegatable_ua_ids(pool.get_ref(), delegator).await;
    if !delegatable.contains(&body.fk_user_attribute) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Target user attribute is not directly held by delegator"
        }));
    }
    // 幂等：同 delegator+delegatee+UA 存在 active 且时间窗重叠 → 409
    let overlap: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_delegation \
         WHERE fk_delegator = $1 AND fk_delegatee = $2 AND fk_user_attribute = $3 \
           AND status = 'active' AND deleted_at IS NULL \
           AND date_st < $4 AND date_ed > $5 LIMIT 1",
    )
    .bind(delegator)
    .bind(body.fk_delegatee)
    .bind(body.fk_user_attribute)
    .bind(body.date_ed)
    .bind(body.date_st)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);
    if overlap.is_some() {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": "Active delegation overlaps for this user attribute and delegatee"
        }));
    }

    let result = sqlx::query_scalar::<_, i64>(
        "INSERT INTO isahl_auth.ngac_delegation \
         (fk_delegator, fk_delegatee, fk_user_attribute, date_st, date_ed) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(delegator)
    .bind(body.fk_delegatee)
    .bind(body.fk_user_attribute)
    .bind(body.date_st)
    .bind(body.date_ed)
    .fetch_one(pool.get_ref())
    .await;
    match result {
        Ok(id) => HttpResponse::Created().json(serde_json::json!({
            "id": id.to_string(),
            "status": "active",
        })),
        Err(e) => {
            log::error!("create_delegation: insert failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create delegation"
            }))
        }
    }
}

/// GET /api/ngac/delegations/me?direction=out|in — 我的委托（服务端过滤）
pub async fn list_my_delegations(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    query: web::Query<DelegationQuery>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let user_id = match jwt_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let (col, _) = match query.direction.as_deref() {
        Some("in") => ("fk_delegatee", "in"),
        _ => ("fk_delegator", "out"),
    };
    let sql = format!(
        "SELECT id, fk_delegator, fk_delegatee, fk_user_attribute, date_st, date_ed, status, created_at \
         FROM isahl_auth.ngac_delegation \
         WHERE {col} = $1 AND deleted_at IS NULL \
         ORDER BY id DESC LIMIT $2 OFFSET $3"
    );
    let rows = sqlx::query_as::<_, DelegationRow>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await;
    match rows {
        Ok(rows) => HttpResponse::Ok().json(rows),
        Err(e) => {
            log::error!("list_my_delegations: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to list delegations"
            }))
        }
    }
}

/// POST /api/ngac/delegations/{id}/revoke — 撤销（delegator 本人或 admin）
pub async fn revoke_delegation(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）
    crate::ngac::ensure::ensure_ngac_extension_tables(pool.get_ref()).await;
    let user_id = match jwt_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let is_admin = crate::admin::handlers::require_admin(&req, pool.get_ref(), state.get_ref())
        .await
        .is_ok();
    let delegation_id = path.into_inner();

    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT fk_delegator, status FROM isahl_auth.ngac_delegation \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(delegation_id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);
    let Some((delegator, status)) = row else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "Delegation not found"
        }));
    };
    if !is_admin && delegator != user_id {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only delegator or admin can revoke"
        }));
    }
    if status != "active" {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": format!("Delegation already {}", status)
        }));
    }
    let result = sqlx::query(
        "UPDATE isahl_auth.ngac_delegation SET status = 'revoked', updated_at = NOW() WHERE id = $1",
    )
    .bind(delegation_id)
    .execute(pool.get_ref())
    .await;
    match result {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({
            "id": delegation_id.to_string(),
            "status": "revoked",
        })),
        Err(e) => {
            log::error!("revoke_delegation: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to revoke delegation"
            }))
        }
    }
}

/// GET /api/ngac/delegations/delegatees — 可选被委托人目录（active 用户，排除本人与 system）
pub async fn list_delegatees(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
) -> HttpResponse {
    let user_id = match jwt_user_id(&req, state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let rows: Result<Vec<(i64, String)>, sqlx::Error> = sqlx::query_as(
        "SELECT id, COALESCE(NULLIF(BTRIM(name), ''), username, id::text) \
         FROM isahl_auth.auth_users \
         WHERE is_active = true AND id <> $1 AND COALESCE(user_type, '') <> 'system' \
         ORDER BY name, id LIMIT 500",
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await;
    match rows {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "delegatees": rows
                .iter()
                .map(|(id, name)| serde_json::json!({"id": id.to_string(), "name": name}))
                .collect::<Vec<_>>()
        })),
        Err(e) => {
            log::error!("list_delegatees: failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to list delegatees"
            }))
        }
    }
}

/// 本人路由（挂载于 /api/ngac，PEP 层豁免 + handler 内 JWT 强制）
pub fn configure_self_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/delegations", web::post().to(create_delegation))
        .route("/delegations/me", web::get().to(list_my_delegations))
        .route("/delegations/delegatees", web::get().to(list_delegatees))
        .route(
            "/delegations/{id}/revoke",
            web::post().to(revoke_delegation),
        );
}
