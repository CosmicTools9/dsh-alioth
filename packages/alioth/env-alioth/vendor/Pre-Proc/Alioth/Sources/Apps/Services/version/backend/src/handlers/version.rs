//! 版本 Handler — 自定义 CRUD（entity 共享内核 + Alioth 契约投影 + RLS/NGAC）
//!
//! Alioth 前端契约字段：tpl_id / version_number(=tk_version) / revision(=reversion) /
//! previous_id(=fk_previous)。投影在壳层完成，entity 面为物理字段并集。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use common::data::{ApiResponse, ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::entity::AliothDbEntity;
use crud::handler::{
    extract_user_id, parse_authorized_columns, parse_visible_ids, register_created_resource_ngac,
    resolve_dk_ctx,
};
use crud::AliothRepository;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use version::entity::{
    CreateVersionRequest, UpdateVersionRequest, VersionRecord, VersionRepository,
};

/// Alioth 版本响应形状
#[derive(Debug, Clone, Serialize)]
pub struct VersionResp {
    pub id: i64,
    pub tpl_id: Option<i64>,
    pub version_number: Option<i64>,
    pub revision: Option<i64>,
    pub previous_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Alioth 版本创建请求（前端契约）
#[derive(Debug, Clone, Deserialize)]
pub struct CreateVersionReq {
    pub tpl_id: Option<i64>,
    pub version_number: Option<i64>,
    pub revision: Option<i64>,
    pub previous_id: Option<i64>,
}

/// Alioth 版本更新请求（前端契约）
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateVersionReq {
    pub tpl_id: Option<i64>,
    pub version_number: Option<i64>,
    pub revision: Option<i64>,
    pub previous_id: Option<i64>,
}

fn to_resp(v: VersionRecord) -> VersionResp {
    VersionResp {
        id: v.id,
        tpl_id: v.tpl_id,
        version_number: v.tk_version,
        revision: v.reversion,
        previous_id: v.fk_previous,
        created_at: v.created_at,
        updated_at: v.updated_at,
        deleted_at: v.deleted_at,
    }
}

fn to_create(req: CreateVersionReq) -> CreateVersionRequest {
    CreateVersionRequest {
        notice: None,
        code: None,
        comments: None,
        tk_version: req.version_number,
        tk_batch_no: None,
        reversion: req.revision,
        fk_previous: req.previous_id,
        ck_branch: None,
        tpl_id: req.tpl_id,
    }
}

fn to_update(req: UpdateVersionReq) -> UpdateVersionRequest {
    UpdateVersionRequest {
        notice: None,
        code: None,
        comments: None,
        tk_version: req.version_number,
        tk_batch_no: None,
        reversion: req.revision,
        fk_previous: req.previous_id,
        ck_branch: None,
        tpl_id: req.tpl_id,
    }
}

async fn list_versions(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError> {
    let repo = VersionRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let page = repo
        .list_with_rls(
            &query,
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?;
    let items: Vec<VersionResp> = page.items.into_iter().map(to_resp).collect();
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(PaginatedResponse {
            page: page.page,
            page_size: page.page_size,
            total: page.total,
            items,
        })),
    )
}

async fn get_version(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = VersionRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let v = repo
        .get_with_rls(
            path.into_inner(),
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?
        .ok_or_else(|| AliothError::NotFound("version".into()))?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(to_resp(v))))
}

async fn create_version(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateVersionReq>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let repo = VersionRepository::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<VersionRecord>(pool.get_ref(), &req).await;
    let created = repo
        .create_with_rls(to_create(body.into_inner()), user_id, dk_ctx.as_ref())
        .await?;
    // Alioth 链维护：旧链头指向新版本（tpl_id 锚点语义）
    repo.link_chain(created.id, created.tpl_id).await?;
    // NGAC 资源注册（与 crud_routes 行为等价，NGAC_SPEC §2.2）
    register_created_resource_ngac::<VersionRecord>(pool.get_ref(), created.id, user_id).await;
    Ok(HttpResponse::Created().json(ApiResponse::success(to_resp(created))))
}

async fn update_version(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
    body: web::Json<UpdateVersionReq>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        VersionRecord::ENTITY_NAME,
        id,
        "update",
    )
    .await?;
    let repo = VersionRepository::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids）：不可见行 -> NotFound
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("version".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<VersionRecord>(pool.get_ref(), &req).await;
    let updated = repo
        .update_with_rls(id, to_update(body.into_inner()), user_id, dk_ctx.as_ref())
        .await?;
    match updated {
        Some(v) => Ok(HttpResponse::Ok().json(ApiResponse::success(to_resp(v)))),
        None => Err(AliothError::NotFound("version".into())),
    }
}

async fn delete_version(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        VersionRecord::ENTITY_NAME,
        id,
        "delete",
    )
    .await?;
    let repo = VersionRepository::from(pool.get_ref().clone());
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("version".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<VersionRecord>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::<()>::success(())))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/versions")
            .route("", web::get().to(list_versions))
            .route("", web::post().to(create_version))
            .route("/{id}", web::get().to(get_version))
            .route("/{id}", web::put().to(update_version))
            .route("/{id}", web::delete().to(delete_version)),
    );
}
