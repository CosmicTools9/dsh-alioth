//! Customer HTTP Handler + 路由注册
//!
//! - GET /customers — 列表（分页 {items,total}，_refs 解析）
//! - POST /customers — 创建（201）
//! - GET /customers/{id} — 单条（_refs）
//! - PUT /customers/{id} — 更新（200）
//! - DELETE /customers/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateCustomerRequest, UpdateCustomerRequest};
use crate::repositories::customer_repository::CustomerRepository;

fn repo(pool: &PgPool) -> CustomerRepository {
    CustomerRepository::new(pool.clone())
}

/// GET /customers
async fn list_customers(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /customers
async fn create_customer(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateCustomerRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /customers/{id}
async fn get_customer(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!("customer {} not found", path))),
    }
}

/// PUT /customers/{id}
async fn update_customer(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateCustomerRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!("customer {} not found", path))),
    }
}

/// DELETE /customers/{id}
async fn delete_customer(
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
        web::scope("/customers")
            .route("", web::get().to(list_customers))
            .route("", web::post().to(create_customer))
            .route("/{id}", web::get().to(get_customer))
            .route("/{id}", web::put().to(update_customer))
            .route("/{id}", web::delete().to(delete_customer)),
    );
}
