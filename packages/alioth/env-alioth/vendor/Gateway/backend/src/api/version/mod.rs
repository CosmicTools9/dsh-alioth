//! Gateway 版本控制通用 Handler
//!
//! 为 zc_id_version 子表提供 REST API Handler。
//! Module 在注册路由时可直接使用本模块的函数。

use actix_web::{web, HttpResponse, Result};
use sqlx::PgPool;
use version::VersionService;

/// 获取版本链
pub async fn list_versions<S: VersionService>(
    service: web::Data<S>,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse> {
    let entity_id = path.into_inner();
    match service.list_versions(pool.get_ref(), entity_id).await {
        Ok(versions) => Ok(HttpResponse::Ok().json(versions)),
        Err(e) => Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "error": e.to_string()
        }))),
    }
}

/// 创建新版本
pub async fn create_version<S: VersionService>(
    service: web::Data<S>,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<CreateVersionRequest>,
) -> Result<HttpResponse> {
    let entity_id = path.into_inner();
    match service
        .create_version(
            pool.get_ref(),
            entity_id,
            body.x_version.clone(),
            body.comment.clone(),
        )
        .await
    {
        Ok(version) => Ok(HttpResponse::Ok().json(version)),
        Err(e) => Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "error": e.to_string()
        }))),
    }
}

/// 回滚到指定版本
pub async fn rollback<S: VersionService>(
    service: web::Data<S>,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse> {
    let (entity_id, target_version_id) = path.into_inner();
    match service
        .rollback(pool.get_ref(), entity_id, target_version_id)
        .await
    {
        Ok(version) => Ok(HttpResponse::Ok().json(version)),
        Err(e) => Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "error": e.to_string()
        }))),
    }
}

#[derive(serde::Deserialize)]
pub struct CreateVersionRequest {
    pub x_version: Option<String>,
    pub comment: Option<String>,
}
