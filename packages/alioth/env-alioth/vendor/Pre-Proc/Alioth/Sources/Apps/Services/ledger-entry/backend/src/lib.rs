//! ledger-entry — ledger-entry 服务（crud 模式后端骨架，gen-service-backend.ts 生成）

pub mod handlers;
pub mod models;
pub mod repositories;

use actix_web::web;

/// 注册全部路由。Gateway 加载时通过 service_registry 调用此函数。
/// scope path 必须为 `/service/{id}`，`id` 与 service.json 的 `"id"` 一致。
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/service/ledger-entry").configure(handlers::register));
}
