//! # alioth-service-measurement — 计量因子壳
//!
//! 全部实现位于 Framework/backend/measurement（统一 CRUD + multiplier 组装 + 补色）。
//! 本模块仅注册 namespace 路由前缀 + 种子数据（初始化数据非实现）。

pub mod seed;

use actix_web::web;

pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/service/measurement").configure(measurement::configure_routes));
}
