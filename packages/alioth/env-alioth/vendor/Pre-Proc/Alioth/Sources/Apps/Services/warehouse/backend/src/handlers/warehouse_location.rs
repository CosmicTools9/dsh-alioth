//! WarehouseLocation HTTP Handler + 路由注册
//!
//! - GET /warehouse-locations — 列表（分页 {items,total}，_refs 解析）
//! - POST /warehouse-locations — 创建（201）
//! - GET /warehouse-locations/{id} — 单条（_refs）
//! - PUT /warehouse-locations/{id} — 更新（200）
//! - DELETE /warehouse-locations/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateWarehouseLocationRequest, UpdateWarehouseLocationRequest};
use crate::repositories::warehouse_location_repository::WarehouseLocationRepository;

fn repo(pool: &PgPool) -> WarehouseLocationRepository {
    WarehouseLocationRepository::new(pool.clone())
}

/// GET /warehouse-locations
async fn list_warehouse_locations(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /warehouse-locations
async fn create_warehouse_location(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateWarehouseLocationRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /warehouse-locations/{id}
async fn get_warehouse_location(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "warehouse_location {} not found",
            path
        ))),
    }
}

/// PUT /warehouse-locations/{id}
async fn update_warehouse_location(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateWarehouseLocationRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "warehouse_location {} not found",
            path
        ))),
    }
}

/// DELETE /warehouse-locations/{id}
async fn delete_warehouse_location(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    repo(pool.get_ref()).delete(*path, user_id).await?;
    Ok(HttpResponse::NoContent().finish())
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/warehouse-locations")
            .route("", web::get().to(list_warehouse_locations))
            .route("", web::post().to(create_warehouse_location))
            .route("/{id}", web::get().to(get_warehouse_location))
            .route("/{id}", web::put().to(update_warehouse_location))
            .route("/{id}", web::delete().to(delete_warehouse_location)),
    );
}
