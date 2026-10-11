//! alioth-service-demand — 需求管理服务（组合薄壳）
//!
//! 实现单源 = Framework `demand` 内核（模型数据链路基础设施；见
//! `openspec/specs/framework-scope-boundary` R1 ② 类 / R2「ns 后端 MUST 为组合薄壳」）。
//! 本壳只保留路由注册；映射 `isahl."zc_id_event"`（继承 `zc_id_lifecycle`）：Requirement 实体 CRUD。
//! 单源判定：change extract-ns-service-kernels-demand-openapi。

use actix_web::web;

/// 注册全部路由。Gateway 加载时通过 service_registry 调用此函数。
/// scope path 必须为 `/service/{id}`，`id` 与 service.json 的 `"id"` 一致（由内核注册）。
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    demand::register_service_routes(cfg);
}
