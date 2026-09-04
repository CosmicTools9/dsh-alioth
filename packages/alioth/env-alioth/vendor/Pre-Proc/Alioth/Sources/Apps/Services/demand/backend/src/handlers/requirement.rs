//! Requirement HTTP Handler + 路由注册
//!
//! - `GET /requirements` — 需求列表（分页，`{items,total}` 契约，含 `_refs` 名称解析）
//! - `POST /requirements` — 创建需求（201）
//! - `GET /requirements/{id}` — 单条需求（含 `_refs`）
//! - `PUT /requirements/{id}` — 更新需求（200）
//! - `DELETE /requirements/{id}` — 软删除（204）
//! - `GET /requirements/dimensions` — 维度选择器数据源（{categories, places}）
//!
//! 响应体为裸 JSON（列表 = `PaginatedResponse{items,total,...}`、详情 = 实体），
//! 不套 `ApiResponse{success,data}` 包装（与前端契约 `{items,total}` 对齐）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateRequirementRequest, DimensionsResponse, UpdateRequirementRequest};
use crate::repositories::requirement_repository::RequirementRepository;

fn repo(pool: &PgPool) -> RequirementRepository {
    RequirementRepository::new(pool.clone())
}

/// GET /requirements
async fn list_requirements(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /requirements
async fn create_requirement(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateRequirementRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /requirements/{id}
async fn get_requirement(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "requirement {} not found",
            path
        ))),
    }
}

/// PUT /requirements/{id}
async fn update_requirement(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateRequirementRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "requirement {} not found",
            path
        ))),
    }
}

/// DELETE /requirements/{id}
async fn delete_requirement(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    repo(pool.get_ref()).delete(*path, user_id).await?;
    Ok(HttpResponse::NoContent().finish())
}

/// GET /requirements/dimensions — 维度选择器数据源
async fn requirement_dimensions(
    pool: web::Data<PgPool>,
) -> Result<web::Json<DimensionsResponse>, ApiError> {
    let r = repo(pool.get_ref());
    let categories = r.list_categories().await?;
    let places = r.list_places().await?;
    Ok(web::Json(DimensionsResponse { categories, places }))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/requirements")
            .route("", web::get().to(list_requirements))
            .route("", web::post().to(create_requirement))
            .route("/dimensions", web::get().to(requirement_dimensions))
            .route("/{id}", web::get().to(get_requirement))
            .route("/{id}", web::put().to(update_requirement))
            .route("/{id}", web::delete().to(delete_requirement)),
    );
}
