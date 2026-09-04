//! demand — 需求管理服务
//!
//! 映射 `isahl."zc_id_event"`（继承 `zc_id_lifecycle`）：Requirement 实体 CRUD。
//! 类目经 `zc_id_lifecycle_r_category` 关联承载；`timeline` 列保持事件流语义（本服务不读写）。

pub mod handlers;
pub mod models;
pub mod repositories;

use actix_web::web;

/// 注册全部路由。Gateway 加载时通过 service_registry 调用此函数。
/// scope path 必须为 `/service/{id}`，`id` 与 service.json 的 `"id"` 一致。
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/service/demand").configure(handlers::register));
}
