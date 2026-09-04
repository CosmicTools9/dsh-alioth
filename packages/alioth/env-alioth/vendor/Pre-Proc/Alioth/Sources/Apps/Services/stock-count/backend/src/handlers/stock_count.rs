//! StockCount HTTP Handler + 路由注册
//!
//! - GET /stock-counts — 列表（分页 {items,total}，_refs 解析）
//! - POST /stock-counts — 创建（201）
//! - GET /stock-counts/{id} — 单条（_refs）
//! - PUT /stock-counts/{id} — 更新（200）
//! - DELETE /stock-counts/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateStockCountRequest, UpdateStockCountRequest};
use crate::repositories::stock_count_repository::StockCountRepository;

fn repo(pool: &PgPool) -> StockCountRepository {
    StockCountRepository::new(pool.clone())
}

/// GET /stock-counts
async fn list_stock_counts(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /stock-counts
async fn create_stock_count(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateStockCountRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /stock-counts/{id}
async fn get_stock_count(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "stock_count {} not found",
            path
        ))),
    }
}

/// PUT /stock-counts/{id}
async fn update_stock_count(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateStockCountRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "stock_count {} not found",
            path
        ))),
    }
}

/// DELETE /stock-counts/{id}
async fn delete_stock_count(
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
        web::scope("/stock-counts")
            .route("", web::get().to(list_stock_counts))
            .route("", web::post().to(create_stock_count))
            .route("/{id}", web::get().to(get_stock_count))
            .route("/{id}", web::put().to(update_stock_count))
            .route("/{id}", web::delete().to(delete_stock_count)),
    );
}
