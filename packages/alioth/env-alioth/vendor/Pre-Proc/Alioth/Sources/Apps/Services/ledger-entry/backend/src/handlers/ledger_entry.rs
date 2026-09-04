//! LedgerEntry HTTP Handler + 路由注册
//!
//! - GET /ledger-entrys — 列表（分页 {items,total}，_refs 解析）
//! - POST /ledger-entrys — 创建（201）
//! - GET /ledger-entrys/{id} — 单条（_refs）
//! - PUT /ledger-entrys/{id} — 更新（200）
//! - DELETE /ledger-entrys/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateLedgerEntryRequest, UpdateLedgerEntryRequest};
use crate::repositories::ledger_entry_repository::LedgerEntryRepository;

fn repo(pool: &PgPool) -> LedgerEntryRepository {
    LedgerEntryRepository::new(pool.clone())
}

/// GET /ledger-entrys
async fn list_ledger_entrys(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /ledger-entrys
async fn create_ledger_entry(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateLedgerEntryRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /ledger-entrys/{id}
async fn get_ledger_entry(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "ledger_entry {} not found",
            path
        ))),
    }
}

/// PUT /ledger-entrys/{id}
async fn update_ledger_entry(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateLedgerEntryRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "ledger_entry {} not found",
            path
        ))),
    }
}

/// DELETE /ledger-entrys/{id}
async fn delete_ledger_entry(
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
        web::scope("/ledger-entrys")
            .route("", web::get().to(list_ledger_entrys))
            .route("", web::post().to(create_ledger_entry))
            .route("/{id}", web::get().to(get_ledger_entry))
            .route("/{id}", web::put().to(update_ledger_entry))
            .route("/{id}", web::delete().to(delete_ledger_entry)),
    );
}
