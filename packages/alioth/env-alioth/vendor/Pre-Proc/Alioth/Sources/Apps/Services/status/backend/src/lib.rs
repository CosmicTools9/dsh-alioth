//! # alioth-service-status — 状态因子壳
//!
//! 全部实现位于 Framework/backend/status（typed CRUD + NGAC 鉴权 + 审计留痕，含
//! statuses 维护面与 status_mapper 统一状态映射）。本模块仅注册 namespace 路由前缀
//! + 种子数据（初始化数据非实现）。

pub mod seed;

use actix_web::web;

pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    // 路径契约：/service/status/statuses*（壳持 `/service/status` 前缀，typed 面持 `/statuses` 子 scope）
    cfg.service(web::scope("/service/status").configure(status::register));
}
