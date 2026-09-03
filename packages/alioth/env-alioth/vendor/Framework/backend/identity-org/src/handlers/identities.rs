//! 主体资质证照 Handler — WZ logistics-wz 承运商资质管理数据源
//!
//! 覆盖：
//! - `GET /identity-categories` — 证照类型字典（zc_id_cate-identity）
//! - `GET /subjects/{id}/identities?expiring_days=` — 主体证照列表（含有效期与到期派生字段）
//! - `POST /subjects/{id}/identities` — 新增证照（事务：identity + segm-date + rr 关联）
//! - `PUT /subjects/{id}/identities/{relId}` — 更新证照（证号/类型/名称/有效期）
//! - `DELETE /subjects/{id}/identities/{relId}` — 删除证照（软删关联 + 实例）
//!
//! 模型语义（零 DDL）：
//! - 证照实例 = `zc_id_identity`（identity=证照号, dname=名称, ck_category→类型字典）
//! - 主体↔证照关联 = `zc_id_entity_rr_identity`（ref_left=主体, ref_right=证照, qk_period→有效期段）
//! - 有效期 = `zc_id_segm-date`（date_st/date_ed）；更新有效期新建段行并切换 qk_period，不改旧段

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::Deserialize;
use sqlx::PgPool;

use crate::models::{IdentityCategory, SubjectIdentityRow};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityListQuery {
    /// 仅返回该天数内到期（含已过期）的证照
    pub expiring_days: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubjectIdentityRequest {
    pub category_code: String,
    pub cert_no: String,
    pub name: String,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    pub comments: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSubjectIdentityRequest {
    pub category_code: Option<String>,
    pub cert_no: Option<String>,
    pub name: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
}

async fn ensure_subject_exists(pool: &PgPool, subject_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_subjects\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!("主体不存在: {}", subject_id)));
    }
    Ok(())
}

/// 证照类型 code → 字典 id；字典缺项时自动创建（幂等，PASSPORT/OTHER 等新类型免手工灌字典）
pub async fn category_id_by_code(pool: &PgPool, code: &str) -> Result<i64, ApiError> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_cate-identity\" WHERE code = $1 AND deleted_at IS NULL",
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if let Some(id) = id {
        return Ok(id);
    }
    // 字典缺项 → 自动创建（notice 用 code 原文；前端 CERT_TYPE_OPTIONS 的 PASSPORT/OTHER 免维护）
    let new_id: i64 = sqlx::query_scalar(
        "INSERT INTO \"isahl\".\"zc_id_cate-identity\" (id, code, notice, created_by_id) \
         VALUES (isahl.gen_next_zuid(), $1, $1, 1) RETURNING id",
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(new_id)
}

/// GET /service/isahl-db/identity-categories — 证照类型字典
pub async fn list_identity_categories(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "read").await?;

    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, code, notice FROM \"isahl\".\"zc_id_cate-identity\" \
         WHERE deleted_at IS NULL ORDER BY id",
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let items: Vec<IdentityCategory> = rows
        .into_iter()
        .map(|(id, code, name)| IdentityCategory { id, code, name })
        .collect();
    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

/// GET /service/isahl-db/subjects/{id}/identities — 主体证照列表
pub async fn list_subject_identities(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    query: web::Query<IdentityListQuery>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;
    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    #[allow(clippy::type_complexity)] // sqlx 行类型
    let rows: Vec<(
        i64,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    )> = sqlx::query_as(
        "SELECT r.id, i.id, i.identity, i.dname, c.code, c.notice, d.date_st, d.date_ed \
         FROM \"isahl\".\"zc_id_entity_rr_identity\" r \
         JOIN \"isahl\".\"zc_id_identity\" i ON i.id = r.ref_right AND i.deleted_at IS NULL \
         LEFT JOIN \"isahl\".\"zc_id_cate-identity\" c ON c.id = i.ck_category AND c.deleted_at IS NULL \
         LEFT JOIN \"isahl\".\"zc_id_segm-date\" d ON d.id = r.qk_period AND d.deleted_at IS NULL \
         WHERE r.ref_left = $1 AND r.deleted_at IS NULL \
         ORDER BY r.id DESC",
    )
    .bind(subject_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let now = Utc::now();
    let items: Vec<SubjectIdentityRow> = rows
        .into_iter()
        .map(
            |(rel_id, identity_id, cert_no, name, ccode, cname, date_st, date_ed)| {
                let days_to_expire = date_ed.map(|ed| (ed - now).num_days());
                let expired = date_ed.is_some_and(|ed| ed < now);
                SubjectIdentityRow {
                    rel_id,
                    identity_id,
                    cert_no,
                    name,
                    category_code: ccode,
                    category_name: cname,
                    valid_from: date_st,
                    valid_to: date_ed,
                    days_to_expire,
                    expired,
                }
            },
        )
        .filter(|row| match query.expiring_days {
            Some(days) => row.days_to_expire.is_some_and(|d| d <= days),
            None => true,
        })
        .collect();
    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

/// POST /service/isahl-db/subjects/{id}/identities — 新增证照（事务）
pub async fn create_subject_identity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<CreateSubjectIdentityRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "create").await?;
    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let body = body.into_inner();
    if body.cert_no.trim().is_empty() {
        return Err(ApiError::BadRequest("证照号不能为空".into()));
    }
    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("证照名称不能为空".into()));
    }
    let category_id = category_id_by_code(pool.get_ref(), &body.category_code).await?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // 1. 证照实例
    let identity_id: i64 = sqlx::query_scalar(
        "INSERT INTO \"isahl\".\"zc_id_identity\" \
         (notice, identity, dname, ck_category, comments, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $6) RETURNING id",
    )
    .bind(body.name.trim())
    .bind(body.cert_no.trim())
    .bind(body.name.trim())
    .bind(category_id)
    .bind(body.comments.clone())
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    // 2. 有效期段（有任一日期才建段）
    let period_id: Option<i64> = if body.valid_from.is_some() || body.valid_to.is_some() {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO \"isahl\".\"zc_id_segm-date\" \
             (notice, date_st, date_ed, created_by_id, updated_by_id) \
             VALUES ($1, $2, $3, $4, $4) RETURNING id",
        )
        .bind(format!("identity-{} validity", identity_id))
        .bind(body.valid_from)
        .bind(body.valid_to)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        Some(id)
    } else {
        None
    };

    // 3. 主体↔证照关联
    let rel_id: i64 = sqlx::query_scalar(
        "INSERT INTO \"isahl\".\"zc_id_entity_rr_identity\" \
         (notice, ref_left, ref_right, qk_period, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $5, $5) RETURNING id",
    )
    .bind(format!("subject-{} identity", subject_id))
    .bind(subject_id)
    .bind(identity_id)
    .bind(period_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "relId": rel_id.to_string(),
            "identityId": identity_id.to_string(),
        }))),
    )
}

/// PUT /service/isahl-db/subjects/{id}/identities/{relId} — 更新证照
pub async fn update_subject_identity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
    body: web::Json<UpdateSubjectIdentityRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (subject_id, rel_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    let body = body.into_inner();
    let rel: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT ref_right, qk_period FROM \"isahl\".\"zc_id_entity_rr_identity\" \
         WHERE id = $1 AND ref_left = $2 AND deleted_at IS NULL",
    )
    .bind(rel_id)
    .bind(subject_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let (identity_id, period_id) =
        rel.ok_or_else(|| ApiError::NotFound(format!("证照关联不存在: {}", rel_id)))?;

    let new_category_id = match &body.category_code {
        Some(code) => Some(category_id_by_code(pool.get_ref(), code).await?),
        None => None,
    };

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // 1. 实例字段（仅更新传入字段）
    if body.cert_no.is_some() || body.name.is_some() || new_category_id.is_some() {
        sqlx::query(
            "UPDATE \"isahl\".\"zc_id_identity\" SET \
             identity = COALESCE($1, identity), \
             dname = COALESCE($2, dname), \
             notice = COALESCE($2, notice), \
             ck_category = COALESCE($3, ck_category), \
             updated_by_id = $4, updated_at = now() \
             WHERE id = $5 AND deleted_at IS NULL",
        )
        .bind(body.cert_no.as_deref().map(str::trim))
        .bind(body.name.as_deref().map(str::trim))
        .bind(new_category_id)
        .bind(user_id)
        .bind(identity_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }

    // 2. 有效期：新建段行并切换 qk_period（不改旧段）
    if body.valid_from.is_some() || body.valid_to.is_some() {
        let new_period_id: i64 = sqlx::query_scalar(
            "INSERT INTO \"isahl\".\"zc_id_segm-date\" \
             (notice, date_st, date_ed, created_by_id, updated_by_id) \
             VALUES ($1, $2, $3, $4, $4) RETURNING id",
        )
        .bind(format!("identity-{} validity", identity_id))
        .bind(body.valid_from)
        .bind(body.valid_to)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        sqlx::query(
            "UPDATE \"isahl\".\"zc_id_entity_rr_identity\" SET qk_period = $1, updated_by_id = $2, updated_at = now() \
             WHERE id = $3 AND deleted_at IS NULL",
        )
        .bind(new_period_id)
        .bind(user_id)
        .bind(rel_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        let _ = period_id; // 旧段保留不改（ref_count 语义由 DB 维护）
    }

    tx.commit().await.map_err(ApiError::from_sqlx)?;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "relId": rel_id.to_string(),
        }))),
    )
}

/// DELETE /service/isahl-db/subjects/{id}/identities/{relId} — 删除证照（软删关联 + 实例）
pub async fn delete_subject_identity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (subject_id, rel_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "delete").await?;

    let identity_id: Option<i64> = sqlx::query_scalar(
        "SELECT ref_right FROM \"isahl\".\"zc_id_entity_rr_identity\" \
         WHERE id = $1 AND ref_left = $2 AND deleted_at IS NULL",
    )
    .bind(rel_id)
    .bind(subject_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let identity_id =
        identity_id.ok_or_else(|| ApiError::NotFound(format!("证照关联不存在: {}", rel_id)))?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    sqlx::query(
        "UPDATE \"isahl\".\"zc_id_entity_rr_identity\" SET deleted_at = now(), deleted_by_id = $1 \
         WHERE id = $2 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(rel_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        "UPDATE \"isahl\".\"zc_id_identity\" SET deleted_at = now(), deleted_by_id = $1 \
         WHERE id = $2 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(identity_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "relId": rel_id.to_string(),
        }))),
    )
}

/// 路由注册（由 handlers/mod.rs 的 configure 调用）
pub fn configure_identities(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/identity-categories").route(web::get().to(list_identity_categories)),
    )
    .service(
        web::resource("/subjects/{id}/identities")
            .route(web::get().to(list_subject_identities))
            .route(web::post().to(create_subject_identity)),
    )
    .service(
        web::resource("/subjects/{id}/identities/{relId}")
            .route(web::put().to(update_subject_identity))
            .route(web::delete().to(delete_subject_identity)),
    );
}
