//! SalesOrder HTTP Handler + 路由注册
//!
//! - GET /sales-orders — 列表（分页 {items,total}，_refs 解析）
//! - POST /sales-orders — 创建（201）
//! - GET /sales-orders/{id} — 单条（_refs）
//! - PUT /sales-orders/{id} — 更新（200）
//! - DELETE /sales-orders/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateSalesOrderRequest, UpdateSalesOrderRequest};
use crate::repositories::sales_order_repository::SalesOrderRepository;

fn repo(pool: &PgPool) -> SalesOrderRepository {
    SalesOrderRepository::new(pool.clone())
}

/// GET /sales-orders
async fn list_sales_orders(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /sales-orders
async fn create_sales_order(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateSalesOrderRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /sales-orders/{id}
async fn get_sales_order(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "sales_order {} not found",
            path
        ))),
    }
}

/// PUT /sales-orders/{id}
async fn update_sales_order(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateSalesOrderRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "sales_order {} not found",
            path
        ))),
    }
}

/// DELETE /sales-orders/{id}
async fn delete_sales_order(
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
        web::scope("/sales-orders")
            .route("", web::get().to(list_sales_orders))
            .route("", web::post().to(create_sales_order))
            .route("/{id}", web::get().to(get_sales_order))
            .route("/{id}", web::put().to(update_sales_order))
            .route("/{id}", web::delete().to(delete_sales_order)),
    );
}
