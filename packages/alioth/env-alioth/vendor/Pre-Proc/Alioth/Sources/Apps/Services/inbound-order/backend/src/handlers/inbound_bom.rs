//! InboundBom HTTP Handler + 路由注册
//!
//! - GET /inbound-boms — 列表（分页 {items,total}，_refs 解析）
//! - POST /inbound-boms — 创建（201）
//! - GET /inbound-boms/{id} — 单条（_refs）
//! - PUT /inbound-boms/{id} — 更新（200）
//! - DELETE /inbound-boms/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateInboundBomRequest, UpdateInboundBomRequest};
use crate::repositories::inbound_bom_repository::InboundBomRepository;

fn repo(pool: &PgPool) -> InboundBomRepository {
    InboundBomRepository::new(pool.clone())
}

/// GET /inbound-boms
async fn list_inbound_boms(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /inbound-boms
async fn create_inbound_bom(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateInboundBomRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /inbound-boms/{id}
async fn get_inbound_bom(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "inbound_bom {} not found",
            path
        ))),
    }
}

/// PUT /inbound-boms/{id}
async fn update_inbound_bom(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateInboundBomRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "inbound_bom {} not found",
            path
        ))),
    }
}

/// DELETE /inbound-boms/{id}
async fn delete_inbound_bom(
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
        web::scope("/inbound-boms")
            .route("", web::get().to(list_inbound_boms))
            .route("", web::post().to(create_inbound_bom))
            .route("/{id}", web::get().to(get_inbound_bom))
            .route("/{id}", web::put().to(update_inbound_bom))
            .route("/{id}", web::delete().to(delete_inbound_bom)),
    );
}
