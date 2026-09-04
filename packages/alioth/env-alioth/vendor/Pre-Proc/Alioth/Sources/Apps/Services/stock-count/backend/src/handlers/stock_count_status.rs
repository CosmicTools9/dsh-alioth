//! StockCountStatus HTTP Handler + 路由注册
//!
//! - GET /stock-count-statuses — 列表（分页 {items,total}，_refs 解析）
//! - POST /stock-count-statuses — 创建（201）
//! - GET /stock-count-statuses/{id} — 单条（_refs）
//! - PUT /stock-count-statuses/{id} — 更新（200）
//! - DELETE /stock-count-statuses/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateStockCountStatusRequest, UpdateStockCountStatusRequest};
use crate::repositories::stock_count_status_repository::StockCountStatusRepository;

fn repo(pool: &PgPool) -> StockCountStatusRepository {
    StockCountStatusRepository::new(pool.clone())
}

/// GET /stock-count-statuses
async fn list_stock_count_statuss(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /stock-count-statuses
async fn create_stock_count_status(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateStockCountStatusRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /stock-count-statuses/{id}
async fn get_stock_count_status(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "stock_count_status {} not found",
            path
        ))),
    }
}

/// PUT /stock-count-statuses/{id}
async fn update_stock_count_status(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateStockCountStatusRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "stock_count_status {} not found",
            path
        ))),
    }
}

/// DELETE /stock-count-statuses/{id}
async fn delete_stock_count_status(
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
        web::scope("/stock-count-statuses")
            .route("", web::get().to(list_stock_count_statuss))
            .route("", web::post().to(create_stock_count_status))
            .route("/{id}", web::get().to(get_stock_count_status))
            .route("/{id}", web::put().to(update_stock_count_status))
            .route("/{id}", web::delete().to(delete_stock_count_status)),
    );
}
