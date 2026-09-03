//! 身份实体 HTTP Handler + 路由注册
//!
//! 演示两种路由注册方式:
//! 1. `crud_routes` 一键注册（标准场景）
//! 2. 手动 handler 注册（需要自定义逻辑时）
//!
//! 其他 namespace 的 Handler 应参照此模块。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ListQuery;
use common::permissions::require_resource_access;
use sqlx::PgPool;

use crate::models::{CreateIdentityRequest, UpdateIdentityRequest};
use crate::service::IdentityService;

/// 注册 identity 相关的全部路由
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/identities")
            .route(web::get().to(list_identities))
            .route(web::post().to(create_identity)),
    )
    .service(
        web::resource("/identities/{id}")
            .route(web::get().to(get_identity))
            .route(web::put().to(update_identity))
            .route(web::delete().to(delete_identity)),
    );
}

/// GET /api/service/identity/identities
async fn list_identities(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, actix_web::Error> {
    // 列表读不做硬 NGAC 检查（对齐 crud_list 与 transport-dispatch 列表 handler；
    // NGAC_SPEC §6 行级安全模型，行过滤由 RLS/PDP visible_ids 承担）。
    // 变更类操作（create/update/delete）仍保留 require_resource_access。
    require_auth(&req)?;
    let service = IdentityService::new(pool.get_ref().clone());
    service
        .list(&query.into_inner())
        .await
        .map(|r| HttpResponse::Ok().json(r))
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// GET /api/service/identity/identities/{id}
async fn get_identity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", id, "read")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    let service = IdentityService::new(pool.get_ref().clone());
    service
        .get(id)
        .await
        .map(|r| match r {
            Some(entity) => HttpResponse::Ok().json(entity),
            None => HttpResponse::NotFound().json(serde_json::json!({
                "error": "not_found", "message": format!("Identity {} not found", id)
            })),
        })
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// POST /api/service/identity/identities
async fn create_identity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateIdentityRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "create")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    let service = IdentityService::new(pool.get_ref().clone());
    service
        .create(body.into_inner(), user_id)
        .await
        .map(|e| HttpResponse::Created().json(e))
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// PUT /api/service/identity/identities/{id}
async fn update_identity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateIdentityRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", id, "update")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    let service = IdentityService::new(pool.get_ref().clone());
    service
        .update(id, body.into_inner(), user_id)
        .await
        .map(|r| match r {
            Some(entity) => HttpResponse::Ok().json(entity),
            None => HttpResponse::NotFound().json(serde_json::json!({
                "error": "not_found", "message": format!("Identity {} not found", id)
            })),
        })
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// DELETE /api/service/identity/identities/{id}
async fn delete_identity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", id, "delete")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    let service = IdentityService::new(pool.get_ref().clone());
    service
        .delete(id, user_id)
        .await
        .map(|_| HttpResponse::NoContent().finish())
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}
