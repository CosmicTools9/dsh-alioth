//! 状态 Handler — 自定义 CRUD（status 共享内核 + Alioth 契约投影 + RLS/NGAC）
//!
//! Alioth 前端契约字段：name（notice AS name 别名）/flag/comments。

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
use status::models::{CreateStatusRequest, Status, UpdateStatusRequest};
use status::StatusRepository;

/// Alioth 状态响应形状（name = notice 别名）
#[derive(Debug, Clone, Serialize)]
pub struct StatusResp {
    pub id: i64,
    pub name: Option<String>,
    pub flag: Option<String>,
    pub comments: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Alioth 状态创建请求（前端契约）
#[derive(Debug, Clone, Deserialize)]
pub struct CreateStatusReq {
    pub name: Option<String>,
    pub flag: Option<String>,
    pub comments: Option<String>,
}

/// Alioth 状态更新请求（前端契约）
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateStatusReq {
    pub name: Option<String>,
    pub flag: Option<String>,
    pub comments: Option<String>,
}

fn to_resp(s: Status) -> StatusResp {
    StatusResp {
        id: s.id,
        name: s.notice,
        flag: s.flag,
        comments: s.comments,
        created_at: s.created_at,
        updated_at: s.updated_at,
        deleted_at: s.deleted_at,
    }
}

fn to_create(req: CreateStatusReq) -> CreateStatusRequest {
    CreateStatusRequest {
        notice: req.name,
        code: None,
        flag: req.flag,
        enable: None,
        comments: req.comments,
    }
}

fn to_update(req: UpdateStatusReq) -> UpdateStatusRequest {
    UpdateStatusRequest {
        notice: req.name,
        code: None,
        flag: req.flag,
        enable: None,
        comments: req.comments,
    }
}

async fn list_statuses(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError> {
    let repo = StatusRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let page = repo
        .list_with_rls(
            &query,
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?;
    let items: Vec<StatusResp> = page.items.into_iter().map(to_resp).collect();
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(PaginatedResponse {
            page: page.page,
            page_size: page.page_size,
            total: page.total,
            items,
        })),
    )
}

async fn get_status(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = StatusRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let s = repo
        .get_with_rls(
            path.into_inner(),
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?
        .ok_or_else(|| AliothError::NotFound("status".into()))?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(to_resp(s))))
}

async fn create_status(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateStatusReq>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let repo = StatusRepository::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<Status>(pool.get_ref(), &req).await;
    let created = repo
        .create_with_rls(to_create(body.into_inner()), user_id, dk_ctx.as_ref())
        .await?;
    // NGAC 资源注册（与 crud_routes 行为等价，NGAC_SPEC §2.2）
    register_created_resource_ngac::<Status>(pool.get_ref(), created.id, user_id).await;
    Ok(HttpResponse::Created().json(ApiResponse::success(to_resp(created))))
}

async fn update_status(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
    body: web::Json<UpdateStatusReq>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        Status::ENTITY_NAME,
        id,
        "update",
    )
    .await?;
    let repo = StatusRepository::from(pool.get_ref().clone());
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("status".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<Status>(pool.get_ref(), &req).await;
    let updated = repo
        .update_with_rls(id, to_update(body.into_inner()), user_id, dk_ctx.as_ref())
        .await?;
    match updated {
        Some(s) => Ok(HttpResponse::Ok().json(ApiResponse::success(to_resp(s)))),
        None => Err(AliothError::NotFound("status".into())),
    }
}

async fn delete_status(
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
        Status::ENTITY_NAME,
        id,
        "delete",
    )
    .await?;
    let repo = StatusRepository::from(pool.get_ref().clone());
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("status".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<Status>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::<()>::success(())))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/statuses")
            .route("", web::get().to(list_statuses))
            .route("", web::post().to(create_status))
            .route("/{id}", web::get().to(get_status))
            .route("/{id}", web::put().to(update_status))
            .route("/{id}", web::delete().to(delete_status)),
    );
}
