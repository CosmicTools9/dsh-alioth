//! 工程师 HTTP Handler + 路由注册
//!
//! 泛型参数 `G: NgacGuard` 控制 NGAC defense-in-depth 行为。
//! 默认 `NoopNgacGuard` 不做权限检查；启用 `ngac-rls` feature 后
//! 可注入 `RlsNgacGuard` 实现完整的 require_resource_access + visible_ids。

use actix_web::{web, HttpRequest, HttpResponse};
use common::data::ListQuery;
use sqlx::PgPool;

use crate::models::{CreateEmployeeRequest, UpdateEmployeeRequest};
use crate::ngac::NgacGuard;
use crate::services::EmployeeService;

/// 注册工程师相关的全部路由
pub fn register<G: NgacGuard + 'static>(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/employees")
            .route(web::get().to(list_employees::<G>))
            .route(web::post().to(create_employee::<G>)),
    )
    .service(
        web::resource("/employees/{id}")
            .route(web::get().to(get_employee::<G>))
            .route(web::patch().to(update_employee::<G>))
            .route(web::delete().to(delete_employee::<G>)),
    );
}

/// GET /api/service/identity/employees
async fn list_employees<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, actix_web::Error> {
    let guard = G::default();
    let visible_ids = guard.visible_ids(&req, "employees");
    let service = EmployeeService::new(pool.get_ref().clone());
    service
        .list_with_rls(&query.into_inner(), visible_ids.as_deref())
        .await
        .map(|r| HttpResponse::Ok().json(r))
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// GET /api/service/identity/employees/{id}
async fn get_employee<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "employees", id, "read")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    let service = EmployeeService::new(pool.get_ref().clone());
    service
        .get(id)
        .await
        .map(|r| match r {
            Some(entity) => HttpResponse::Ok().json(entity),
            None => HttpResponse::NotFound().json(serde_json::json!({
                "error": "not_found",
                "message": format!("Employee {} not found", id)
            })),
        })
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// POST /api/service/identity/employees
async fn create_employee<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    body: web::Json<CreateEmployeeRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let guard = G::default();
    // create 使用 resource_id=0 触发"无关联也能用"路径
    guard
        .check_access(pool.get_ref(), user_id, "employees", 0, "write")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    let service = EmployeeService::new(pool.get_ref().clone());
    service
        .create(body.into_inner(), user_id)
        .await
        .map(|e| HttpResponse::Created().json(e))
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// PATCH /api/service/identity/employees/{id}
async fn update_employee<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateEmployeeRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "employees", id, "write")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    let service = EmployeeService::new(pool.get_ref().clone());
    service
        .update(id, body.into_inner(), user_id)
        .await
        .map(|r| match r {
            Some(entity) => HttpResponse::Ok().json(entity),
            None => HttpResponse::NotFound().json(serde_json::json!({
                "error": "not_found",
                "message": format!("Employee {} not found", id)
            })),
        })
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// DELETE /api/service/identity/employees/{id}
async fn delete_employee<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "employees", id, "delete")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    let service = EmployeeService::new(pool.get_ref().clone());
    service
        .delete(id, user_id)
        .await
        .map(|_| HttpResponse::NoContent().finish())
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}
