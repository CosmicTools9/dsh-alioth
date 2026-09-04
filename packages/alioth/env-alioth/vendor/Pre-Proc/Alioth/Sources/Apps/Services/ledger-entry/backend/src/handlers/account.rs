//! Account HTTP Handler + 路由注册
//!
//! - GET /accounts — 列表（分页 {items,total}，_refs 解析）
//! - POST /accounts — 创建（201）
//! - GET /accounts/{id} — 单条（_refs）
//! - PUT /accounts/{id} — 更新（200）
//! - DELETE /accounts/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateAccountRequest, UpdateAccountRequest};
use crate::repositories::account_repository::AccountRepository;

fn repo(pool: &PgPool) -> AccountRepository {
    AccountRepository::new(pool.clone())
}

/// GET /accounts
async fn list_accounts(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /accounts
async fn create_account(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateAccountRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /accounts/{id}
async fn get_account(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!("account {} not found", path))),
    }
}

/// PUT /accounts/{id}
async fn update_account(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateAccountRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!("account {} not found", path))),
    }
}

/// DELETE /accounts/{id}
async fn delete_account(
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
        web::scope("/accounts")
            .route("", web::get().to(list_accounts))
            .route("", web::post().to(create_account))
            .route("/{id}", web::get().to(get_account))
            .route("/{id}", web::put().to(update_account))
            .route("/{id}", web::delete().to(delete_account)),
    );
}
