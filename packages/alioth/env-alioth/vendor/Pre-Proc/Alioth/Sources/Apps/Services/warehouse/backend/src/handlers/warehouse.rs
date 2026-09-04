//! Warehouse HTTP Handler + 路由注册
//!
//! - GET /warehouses — 列表（分页 {items,total}，_refs 解析）
//! - POST /warehouses — 创建（201）
//! - GET /warehouses/{id} — 单条（_refs）
//! - PUT /warehouses/{id} — 更新（200）
//! - DELETE /warehouses/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateWarehouseRequest, UpdateWarehouseRequest};
use crate::repositories::warehouse_repository::WarehouseRepository;

fn repo(pool: &PgPool) -> WarehouseRepository {
    WarehouseRepository::new(pool.clone())
}

/// GET /warehouses
async fn list_warehouses(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /warehouses
async fn create_warehouse(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateWarehouseRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /warehouses/{id}
async fn get_warehouse(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!("warehouse {} not found", path))),
    }
}

/// PUT /warehouses/{id}
async fn update_warehouse(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateWarehouseRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!("warehouse {} not found", path))),
    }
}

/// DELETE /warehouses/{id}
async fn delete_warehouse(
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
        web::scope("/warehouses")
            .route("", web::get().to(list_warehouses))
            .route("", web::post().to(create_warehouse))
            .route("/{id}", web::get().to(get_warehouse))
            .route("/{id}", web::put().to(update_warehouse))
            .route("/{id}", web::delete().to(delete_warehouse)),
    );
}
