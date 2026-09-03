//! 库存统计 HTTP 路由（通用；/balances 只读列表端点）

use actix_web::{web, HttpResponse};
use common::AliothError as ApiError;
use sqlx::PgPool;
use std::sync::Arc;

use crate::models::{BalanceListQuery, NameResolver};
use crate::service::InventoryService;

/// 注册通用库存统计路由（挂载点由 namespace 壳决定，如 /service/inventory-balance）
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("").route("/balances", web::get().to(list_balances)));
}

/// GET /balances?page=&page_size=&sort=&production_id=&storage_id=
///
/// 只读统计端点（无写路径）：库存 = 货在储元中的时空切片数量统计。
async fn list_balances(
    pool: web::Data<PgPool>,
    resolver: web::Data<Arc<dyn NameResolver>>,
    query: web::Query<BalanceListQuery>,
) -> Result<HttpResponse, ApiError> {
    let svc = InventoryService::new(pool.get_ref().clone(), resolver.get_ref().clone());
    let page = svc
        .list(&query.base, query.production_id, query.storage_id)
        .await?;
    Ok(HttpResponse::Ok().json(page))
}
