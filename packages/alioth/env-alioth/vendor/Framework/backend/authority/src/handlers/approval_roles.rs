//! 审批岗位 HTTP Handler + 路由注册
//!
//! 泛型参数 `G: NgacGuard` 控制 NGAC defense-in-depth 行为。

use actix_web::{web, HttpRequest, HttpResponse};
use common::data::ListQuery;
use sqlx::PgPool;

use crate::models::{CreateApprovalRoleRequest, UpdateApprovalRoleRequest};
use crate::ngac::NgacGuard;
use crate::services::ApprovalRoleService;

pub fn register<G: NgacGuard + 'static>(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/approval-roles")
            .route(web::get().to(list_approval_roles::<G>))
            .route(web::post().to(create_approval_role::<G>)),
    )
    .service(
        web::resource("/approval-roles/{id}")
            .route(web::get().to(get_approval_role::<G>))
            .route(web::patch().to(update_approval_role::<G>))
            .route(web::delete().to(delete_approval_role::<G>)),
    );
}

async fn list_approval_roles<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, actix_web::Error> {
    let guard = G::default();
    let visible_ids = guard.visible_ids(&req, "approval-roles");
    ApprovalRoleService::new(pool.get_ref().clone())
        .list_with_rls(&query.into_inner(), visible_ids.as_deref())
        .await
        .map(|r| HttpResponse::Ok().json(r))
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

async fn get_approval_role<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "approval-roles", id, "read")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    ApprovalRoleService::new(pool.get_ref().clone())
        .get(id)
        .await
        .map(|r| match r {
            Some(entity) => HttpResponse::Ok().json(entity),
            None => HttpResponse::NotFound().json(serde_json::json!({
                "error": "not_found",
                "message": format!("ApprovalRole {} not found", id)
            })),
        })
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

async fn create_approval_role<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    body: web::Json<CreateApprovalRoleRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "approval-roles", 0, "write")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    ApprovalRoleService::new(pool.get_ref().clone())
        .create(body.into_inner(), user_id)
        .await
        .map(|e| HttpResponse::Created().json(e))
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

async fn update_approval_role<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateApprovalRoleRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "approval-roles", id, "write")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    ApprovalRoleService::new(pool.get_ref().clone())
        .update(id, body.into_inner(), user_id)
        .await
        .map(|r| match r {
            Some(entity) => HttpResponse::Ok().json(entity),
            None => HttpResponse::NotFound().json(serde_json::json!({
                "error": "not_found",
                "message": format!("ApprovalRole {} not found", id)
            })),
        })
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

async fn delete_approval_role<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "approval-roles", id, "delete")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    ApprovalRoleService::new(pool.get_ref().clone())
        .delete(id, user_id)
        .await
        .map(|_| HttpResponse::NoContent().finish())
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}
