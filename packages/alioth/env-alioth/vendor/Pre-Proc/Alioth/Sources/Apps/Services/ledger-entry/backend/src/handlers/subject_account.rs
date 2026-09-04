//! SubjectAccount HTTP Handler + 路由注册
//!
//! - GET /subject-accounts — 列表（分页 {items,total}，_refs 解析）
//! - POST /subject-accounts — 创建（201）
//! - GET /subject-accounts/{id} — 单条（_refs）
//! - PUT /subject-accounts/{id} — 更新（200）
//! - DELETE /subject-accounts/{id} — 软删除（204）

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::error::AliothError as ApiError;
use crud::{AliothRepository, ListQuery};
use sqlx::PgPool;

use crate::models::{CreateSubjectAccountRequest, UpdateSubjectAccountRequest};
use crate::repositories::subject_account_repository::SubjectAccountRepository;

fn repo(pool: &PgPool) -> SubjectAccountRepository {
    SubjectAccountRepository::new(pool.clone())
}

/// GET /subject-accounts
async fn list_subject_accounts(
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, ApiError> {
    let page = repo(pool.get_ref()).list(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// POST /subject-accounts
async fn create_subject_account(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateSubjectAccountRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    let item = repo(pool.get_ref())
        .create(body.into_inner(), user_id)
        .await?;
    Ok(HttpResponse::Created().json(item))
}

/// GET /subject-accounts/{id}
async fn get_subject_account(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    match repo(pool.get_ref()).get(*path).await? {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "subject_account {} not found",
            path
        ))),
    }
}

/// PUT /subject-accounts/{id}
async fn update_subject_account(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateSubjectAccountRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| ApiError::Unauthorized("Authentication required".to_string()))?;
    match repo(pool.get_ref())
        .update(*path, body.into_inner(), user_id)
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(item)),
        None => Err(ApiError::NotFound(format!(
            "subject_account {} not found",
            path
        ))),
    }
}

/// DELETE /subject-accounts/{id}
async fn delete_subject_account(
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
        web::scope("/subject-accounts")
            .route("", web::get().to(list_subject_accounts))
            .route("", web::post().to(create_subject_account))
            .route("/{id}", web::get().to(get_subject_account))
            .route("/{id}", web::put().to(update_subject_account))
            .route("/{id}", web::delete().to(delete_subject_account)),
    );
}
