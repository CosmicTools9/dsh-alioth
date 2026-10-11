//! OpenAPI 数据服务产品 Handlers — 标准 CRUD 路由
//!
//! 每个实体一个 register fn，挂在 /service/openapi/{entity} scope 下。

use actix_web::web;
use common::AliothError as ApiError;
use crud::crud_routes;

use crate::models::*;
use crate::repositories::*;

/// 注册 OpenAPI 数据服务产品全部路由。
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/service/openapi")
            .configure(register_config)
            .configure(register_sales)
            .configure(register_purchase)
            .configure(register_made),
    );
}

/// 对接配置 CRUD
fn register_config(cfg: &mut web::ServiceConfig) {
    cfg.configure(crud_routes::<
        OpenApiConfig,
        CreateOpenApiConfigRequest,
        UpdateOpenApiConfigRequest,
        OpenApiConfigRepository,
        ApiError,
    >("/configs"));
}

/// 销售侧数据服务产品 CRUD
fn register_sales(cfg: &mut web::ServiceConfig) {
    cfg.configure(crud_routes::<
        OpenApiSales,
        CreateOpenApiSalesRequest,
        UpdateOpenApiSalesRequest,
        OpenApiSalesRepository,
        ApiError,
    >("/sales"));
}

/// 采购侧数据服务产品 CRUD
fn register_purchase(cfg: &mut web::ServiceConfig) {
    cfg.configure(crud_routes::<
        OpenApiPurchase,
        CreateOpenApiPurchaseRequest,
        UpdateOpenApiPurchaseRequest,
        OpenApiPurchaseRepository,
        ApiError,
    >("/purchases"));
}

/// 制造侧数据服务产品 CRUD
fn register_made(cfg: &mut web::ServiceConfig) {
    cfg.configure(crud_routes::<
        OpenApiMade,
        CreateOpenApiMadeRequest,
        UpdateOpenApiMadeRequest,
        OpenApiMadeRepository,
        ApiError,
    >("/mades"));
}
